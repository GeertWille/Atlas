//! Accepting a consent dialog for the scan. The dialog is looked for in
//! every frame of the tab (`injected/consent.js`), because consent platforms
//! serve it from an origin of their own as often as not; what to press is
//! decided there, by a table of platforms and of labels that plainly mean
//! "accept all".
//!
//! The control is pressed the way a reader would press it: a real click at
//! its place on the screen, once the browser confirms that the control's
//! frame is what is at that place. Where that cannot be established the
//! control is clicked from script instead.

use std::time::Duration;

use adscan_model::ConsentNoteKind;
use serde::Deserialize;
use serde_json::{Value, json};

use super::error::CaptureError;
use super::injected::{ConsentAsk, consent_expression};
use super::page::{Page, tolerate_rejection};
use super::recorder::epoch_ms_now;
use super::visitor::Point;
use super::visitor::motion::Waypoint;

/// An ad-heavy page has dozens of frames; the dialog is in one of the first.
const MAX_FRAMES: usize = 40;
const CLICK_HELD: Duration = Duration::from_millis(70);
/// How long the control is watched for the scanner's click to arrive.
const CLICK_ARRIVES_WITHIN_MS: u32 = 1_000;
/// A dialog's container is on the page before its frame has drawn the
/// controls. Only a dialog that still has no accept control after this many
/// looks in a row is one the scanner cannot accept.
const LOOKS_BEFORE_GIVING_UP: u32 = 3;

/// What `injected/consent.js` found in one frame.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
enum Sighting {
    /// The accept-all control, and a point on it in its frame's viewport
    /// when nothing is in front of it there.
    Accept {
        name: String,
        x: Option<f64>,
        y: Option<f64>,
    },
    /// A consent dialog without a control that plainly means "accept all".
    Dialog { dialog: String },
}

/// One frame of the tab and the session it is reached through.
#[derive(Debug, Clone, PartialEq)]
struct Frame {
    session_id: String,
    frame_id: String,
    /// The main frame of the document tree its session shows.
    session_root: bool,
    /// The frame id of the out-of-process iframe the frame lives in, when it
    /// is not part of the tab's own process.
    isolated_in: Option<String>,
}

/// The frames of one session, the main frame first. `frame_tree` is the
/// result of `Page.getFrameTree`.
fn frames_of(session_id: &str, frame_tree: &Value, in_tab: bool) -> Vec<Frame> {
    fn collect(node: &Value, ids: &mut Vec<String>) {
        if let Some(id) = node["frame"]["id"].as_str() {
            ids.push(id.to_string());
        }
        for child in node["childFrames"].as_array().into_iter().flatten() {
            collect(child, ids);
        }
    }
    let mut ids = Vec::new();
    collect(&frame_tree["frameTree"], &mut ids);
    let root = ids.first().cloned();
    ids.into_iter()
        .enumerate()
        .map(|(index, frame_id)| Frame {
            session_id: session_id.to_string(),
            frame_id,
            session_root: index == 0,
            isolated_in: root.clone().filter(|_| !in_tab),
        })
        .collect()
}

impl Page {
    /// Looks for a consent dialog in every frame and accepts it. Returns the
    /// name of what was accepted.
    pub(crate) async fn accept_consent(&mut self) -> Result<Option<String>, CaptureError> {
        let mut dialog_without_accept = None;
        for frame in self.frames().await? {
            match self.look_for_consent(&frame).await? {
                Some(Sighting::Accept { name, x, y }) => {
                    let point = x.zip(y).map(|(x, y)| Point { x, y });
                    self.press(&frame, point).await?;
                    self.recorder.dialog_without_accept_seen = 0;
                    self.recorder
                        .note_consent(ConsentNoteKind::Accepted, name.clone());
                    return Ok(Some(name));
                }
                // The page may hold the dialog's container and a frame
                // further on its accept control.
                Some(Sighting::Dialog { dialog }) => {
                    dialog_without_accept.get_or_insert(dialog);
                }
                None => {}
            }
        }
        let Some(dialog) = dialog_without_accept else {
            self.recorder.dialog_without_accept_seen = 0;
            return Ok(None);
        };
        self.recorder.dialog_without_accept_seen += 1;
        if self.recorder.dialog_without_accept_seen >= LOOKS_BEFORE_GIVING_UP {
            let message = format!(
                "A consent dialog ({dialog}) is showing and nothing in it plainly means \"accept all\"; \
                 it was left alone"
            );
            if self
                .recorder
                .note_consent(ConsentNoteKind::DialogWithoutAccept, dialog)
            {
                self.report_blocked("consent", &message, None);
            }
        }
        Ok(None)
    }

    /// Every frame of the recorded tab: its own, then those the browser
    /// keeps in other processes.
    async fn frames(&mut self) -> Result<Vec<Frame>, CaptureError> {
        let Some((tab_session, frame_sessions)) = self
            .tabs
            .recorded()
            .map(|tab| (tab.session_id.clone(), tab.frame_sessions.clone()))
        else {
            return Err(CaptureError::TabClosed);
        };
        let mut frames = Vec::new();
        for (session_id, in_tab) in std::iter::once((tab_session, true))
            .chain(frame_sessions.into_iter().map(|session| (session, false)))
        {
            if frames.len() >= MAX_FRAMES {
                break;
            }
            let tree = self
                .call_in(&session_id, "Page.getFrameTree", json!({}))
                .await;
            if let Some(tree) = tolerate_rejection(tree)? {
                frames.extend(frames_of(&session_id, &tree, in_tab));
            }
        }
        frames.truncate(MAX_FRAMES);
        Ok(frames)
    }

    async fn look_for_consent(&mut self, frame: &Frame) -> Result<Option<Sighting>, CaptureError> {
        let seen = self.ask_consent_script(frame, ConsentAsk::Look).await?;
        Ok(seen.and_then(|seen| serde_json::from_value(seen).ok()))
    }

    async fn ask_consent_script(
        &mut self,
        frame: &Frame,
        ask: ConsentAsk,
    ) -> Result<Option<Value>, CaptureError> {
        let expression = consent_expression(ask);
        self.evaluate_in_frame(&frame.session_id, &frame.frame_id, &expression)
            .await
    }

    /// Presses the accept control of `frame`: with the pointer when its
    /// place on the screen is known and the frame is what is there, and from
    /// script when it is not, or when the control did not get the click (a
    /// frame the browser has only just created is not always ready to take
    /// input).
    async fn press(
        &mut self,
        frame: &Frame,
        point_in_frame: Option<Point>,
    ) -> Result<(), CaptureError> {
        let on_screen = match point_in_frame {
            Some(point) => self.place_on_screen(frame, point).await?,
            None => None,
        };
        let clicked = match on_screen {
            Some(at) => self.click_and_confirm(frame, at).await?,
            None => false,
        };
        if !clicked {
            self.ask_consent_script(frame, ConsentAsk::Click).await?;
        }
        Ok(())
    }

    /// Clicks at `at` and returns whether the accept control of `frame` saw
    /// the click.
    async fn click_and_confirm(&mut self, frame: &Frame, at: Point) -> Result<bool, CaptureError> {
        let watch = consent_expression(ConsentAsk::WatchForClick(CLICK_ARRIVES_WITHIN_MS));
        let Some(evaluation) = self
            .evaluation_in_frame(&frame.session_id, &frame.frame_id, &watch)
            .await?
        else {
            return Ok(false);
        };
        // Sent now and answered once the click arrived or the time is up.
        let watching = self
            .cdp
            .send(Some(&frame.session_id), "Runtime.evaluate", evaluation);
        if self.emulation().touch {
            self.tap(at, CLICK_HELD).await?;
        } else {
            let there = Waypoint {
                point: at,
                at_ms: 0.0,
            };
            self.move_pointer(&[there]).await?;
            self.click(at, CLICK_HELD).await?;
        }
        let watched = self.record_until_answered(watching).await;
        Ok(tolerate_rejection(watched)?.is_some_and(|watched| watched["result"]["value"] == true))
    }

    /// Where a point of a frame's viewport is in the tab's viewport. `None`
    /// unless the browser confirms that a click there lands in that frame.
    async fn place_on_screen(
        &mut self,
        frame: &Frame,
        point: Point,
    ) -> Result<Option<Point>, CaptureError> {
        let Some(tab_session) = self.tabs.recorded().map(|tab| tab.session_id.clone()) else {
            return Err(CaptureError::TabClosed);
        };
        let mut at = point;
        if !frame.session_root {
            let Some((corner, _)) = self
                .frame_corner(&frame.session_id, &frame.frame_id)
                .await?
            else {
                return Ok(None);
            };
            at = at.offset_by(corner);
        }
        let mut owner_in_tab = None;
        if let Some(isolated_frame) = &frame.isolated_in {
            let Some((corner, owner)) = self.frame_corner(&tab_session, isolated_frame).await?
            else {
                return Ok(None);
            };
            at = at.offset_by(corner);
            owner_in_tab = Some(owner);
        }
        let hit = self
            .call_in(
                &tab_session,
                "DOM.getNodeForLocation",
                json!({ "x": at.x.round(), "y": at.y.round() }),
            )
            .await;
        let Some(hit) = tolerate_rejection(hit)? else {
            return Ok(None);
        };
        // The tab's own frames are named by the hit; a frame in another
        // process shows as the iframe element that holds it.
        let lands_in_the_frame = match owner_in_tab {
            Some(owner) => hit["backendNodeId"] == owner,
            None => hit["frameId"] == frame.frame_id.as_str(),
        };
        Ok(lands_in_the_frame.then_some(at))
    }

    /// The top-left corner of a frame's content in the viewport of
    /// `session_id`, where the frame's `<iframe>` element lives, and that
    /// element's backend node id.
    async fn frame_corner(
        &mut self,
        session_id: &str,
        frame_id: &str,
    ) -> Result<Option<(Point, Value)>, CaptureError> {
        let owner = self
            .call_in(
                session_id,
                "DOM.getFrameOwner",
                json!({ "frameId": frame_id }),
            )
            .await;
        let Some(owner) = tolerate_rejection(owner)?.map(|mut owner| owner["backendNodeId"].take())
        else {
            return Ok(None);
        };
        let quads = self
            .call_in(
                session_id,
                "DOM.getContentQuads",
                json!({ "backendNodeId": owner }),
            )
            .await;
        let corner = tolerate_rejection(quads)?.and_then(|quads| {
            let quad = &quads["quads"][0];
            Some(Point {
                x: quad[0].as_f64()?,
                y: quad[1].as_f64()?,
            })
        });
        Ok(corner.map(|corner| (corner, owner)))
    }
}

impl super::recorder::Recorder {
    /// Remembers what the scanner saw or did about a consent dialog. A
    /// dialog it could not accept is noted once per page view. Returns
    /// whether this was news.
    pub(crate) fn note_consent(&mut self, kind: ConsentNoteKind, banner: String) -> bool {
        let page = self.pages.current();
        if kind == ConsentNoteKind::Accepted {
            // It could be accepted after all.
            self.consent_notes.retain(|note| note.page != page);
        }
        let known = kind == ConsentNoteKind::DialogWithoutAccept
            && self
                .consent_notes
                .iter()
                .any(|note| note.kind == kind && note.page == page);
        if !known {
            self.consent_notes.push(super::recorder::ConsentSeen {
                at_epoch_ms: epoch_ms_now(),
                page,
                kind,
                banner,
            });
        }
        !known
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_a_frame_reports_is_an_accept_control_a_bare_dialog_or_nothing() {
        let parse = |reported: Value| serde_json::from_value::<Sighting>(reported).ok();
        assert_eq!(
            parse(json!({ "name": "Sourcepoint", "x": 120.5, "y": 80 })),
            Some(Sighting::Accept {
                name: "Sourcepoint".into(),
                x: Some(120.5),
                y: Some(80.0)
            })
        );
        assert_eq!(
            parse(json!({ "name": "OneTrust", "x": null, "y": null })),
            Some(Sighting::Accept {
                name: "OneTrust".into(),
                x: None,
                y: None
            })
        );
        assert_eq!(
            parse(json!({ "dialog": "Didomi" })),
            Some(Sighting::Dialog {
                dialog: "Didomi".into()
            })
        );
        assert_eq!(parse(Value::Null), None);
    }

    #[test]
    fn the_frames_of_a_session_are_listed_main_frame_first() {
        let tree = json!({ "frameTree": {
            "frame": { "id": "MAIN" },
            "childFrames": [
                { "frame": { "id": "CMP" }, "childFrames": [{ "frame": { "id": "INNER" } }] },
                { "frame": { "id": "AD" } },
            ],
        }});
        let in_tab = frames_of("tab", &tree, true);
        let ids: Vec<&str> = in_tab.iter().map(|frame| frame.frame_id.as_str()).collect();
        assert_eq!(ids, ["MAIN", "CMP", "INNER", "AD"]);
        assert!(in_tab[0].session_root && !in_tab[1].session_root);
        assert!(in_tab.iter().all(|frame| frame.isolated_in.is_none()));

        let isolated = frames_of("iframe-session", &tree, false);
        assert!(
            isolated
                .iter()
                .all(|frame| frame.isolated_in.as_deref() == Some("MAIN"))
        );
        assert!(frames_of("tab", &json!({}), true).is_empty());
    }
}
