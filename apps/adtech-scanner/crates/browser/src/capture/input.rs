//! Input to the page: the pointer, the wheel and a finger. This is the only
//! place that talks to the browser's `Input` domain, and it has no way to
//! press a key or enter text: a scan never types into a page.
//!
//! The events are real input to the browser, so the page sees what a reader's
//! devices would cause, timed as planned in `visitor::motion`.

use std::time::Duration;

use serde_json::{Value, json};
use tokio::time::Instant;

use super::error::CaptureError;
use super::page::{Page, tolerate_rejection};
use super::scenario::swipe_lengths;
use super::visitor::Point;
use super::visitor::motion::{Waypoint, WheelTick};

/// The finger rests before it lifts: lifting it in motion starts a fling of
/// unknown length.
const SWIPE_REST: Duration = Duration::from_millis(80);

/// The plain scroll of the `scroll` behaviour: where on the screen a swipe
/// starts, as a share of the viewport height, and how it is paced.
const PLAIN_SWIPE_START: f64 = 0.8;
const PLAIN_SWIPE_FRAMES: u32 = 12;
const PLAIN_SWIPE_FRAME_MS: f64 = 16.0;

impl Page {
    /// Moves the pointer along `path`.
    pub(crate) async fn move_pointer(&mut self, path: &[Waypoint]) -> Result<(), CaptureError> {
        let started = Instant::now();
        for waypoint in path {
            self.record_until(started + Duration::from_secs_f64(waypoint.at_ms / 1_000.0))
                .await?;
            self.mouse(
                "mouseMoved",
                waypoint.point,
                json!({ "button": "none", "buttons": 0 }),
            )
            .await?;
            self.pointer = waypoint.point;
        }
        Ok(())
    }

    /// Turns the wheel where the pointer is.
    pub(crate) async fn wheel(&mut self, ticks: &[WheelTick]) -> Result<(), CaptureError> {
        for tick in ticks {
            self.record_for(Duration::from_secs_f64(tick.gap_ms / 1_000.0))
                .await?;
            self.mouse(
                "mouseWheel",
                self.pointer,
                json!({ "deltaX": 0, "deltaY": tick.delta_y }),
            )
            .await?;
        }
        Ok(())
    }

    /// Presses and releases the main button at `at`, where the pointer has
    /// to be already.
    pub(crate) async fn click(
        &mut self,
        at: Point,
        held_for: Duration,
    ) -> Result<(), CaptureError> {
        let button =
            |buttons: u32| json!({ "button": "left", "buttons": buttons, "clickCount": 1 });
        self.mouse("mousePressed", at, button(1)).await?;
        self.record_for(held_for).await?;
        self.mouse("mouseReleased", at, button(0)).await
    }

    async fn mouse(&mut self, kind: &str, at: Point, more: Value) -> Result<(), CaptureError> {
        let mut event = json!({ "type": kind, "x": at.x, "y": at.y });
        if let (Some(event), Some(more)) = (event.as_object_mut(), more.as_object()) {
            event.extend(more.clone());
        }
        tolerate_rejection(self.call("Input.dispatchMouseEvent", event).await).map(drop)
    }

    /// One finger down at the start of `path`, along it, and up at its end.
    pub(crate) async fn swipe(&mut self, path: &[Waypoint]) -> Result<(), CaptureError> {
        let Some((first, rest)) = path.split_first() else {
            return Ok(());
        };
        let started = Instant::now();
        self.touch("touchStart", Some(first.point)).await?;
        for waypoint in rest {
            self.record_until(started + Duration::from_secs_f64(waypoint.at_ms / 1_000.0))
                .await?;
            self.touch("touchMove", Some(waypoint.point)).await?;
        }
        self.record_for(SWIPE_REST).await?;
        self.touch("touchEnd", None).await
    }

    /// One finger down and up at `at`.
    pub(crate) async fn tap(&mut self, at: Point, held_for: Duration) -> Result<(), CaptureError> {
        self.touch("touchStart", Some(at)).await?;
        self.record_for(held_for).await?;
        self.touch("touchEnd", None).await
    }

    async fn touch(&mut self, kind: &str, finger: Option<Point>) -> Result<(), CaptureError> {
        let points: Vec<Value> = finger
            .iter()
            .map(|at| json!({ "x": at.x, "y": at.y }))
            .collect();
        let event = json!({ "type": kind, "touchPoints": points });
        tolerate_rejection(self.call("Input.dispatchTouchEvent", event).await).map(drop)
    }

    /// The plain scroll of the `scroll` behaviour: `distance` pixels down, by
    /// finger swipes on a touch device and one turn of the wheel otherwise.
    pub(crate) async fn scroll_down(&mut self, distance: f64) -> Result<(), CaptureError> {
        let viewport = self.emulated_viewport();
        let middle = Point {
            x: viewport.width / 2.0,
            y: viewport.height / 2.0,
        };
        if !self.emulation().touch {
            let wheel = json!({ "deltaX": 0, "deltaY": distance });
            return self.mouse("mouseWheel", middle, wheel).await;
        }
        for length in swipe_lengths(distance, viewport.height) {
            let from_y = viewport.height * PLAIN_SWIPE_START;
            let path: Vec<Waypoint> = (0..=PLAIN_SWIPE_FRAMES)
                .map(|frame| Waypoint {
                    point: Point {
                        x: middle.x,
                        y: from_y - length * f64::from(frame) / f64::from(PLAIN_SWIPE_FRAMES),
                    },
                    at_ms: PLAIN_SWIPE_FRAME_MS * f64::from(frame),
                })
                .collect();
            self.swipe(&path).await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    fn sources_under(directory: &Path, found: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(directory).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                sources_under(&path, found);
            } else {
                found.push(path);
            }
        }
    }

    /// Everything the scanner can make the browser do, as source text: its
    /// own code and the scripts it injects.
    fn sources() -> Vec<(PathBuf, String)> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut files = Vec::new();
        sources_under(&root.join("src"), &mut files);
        sources_under(&root.join("injected"), &mut files);
        files
            .into_iter()
            .map(|path| {
                let text = std::fs::read_to_string(&path).unwrap();
                (path, text)
            })
            .collect()
    }

    #[test]
    fn nothing_in_the_scanner_can_type_into_a_page() {
        // Spelled in pieces, so that this test does not find itself.
        let keyboard_commands = [
            ["dispatch", "KeyEvent"].concat(),
            ["insert", "Text"].concat(),
            ["ime", "SetComposition"].concat(),
        ];
        let sources = sources();
        assert!(sources.len() > 20, "found {} files", sources.len());
        for (path, text) in &sources {
            for command in &keyboard_commands {
                assert!(!text.contains(command), "{} uses {command}", path.display());
            }
        }
    }

    #[test]
    fn input_is_sent_from_this_file_only() {
        let domain = ["\"Input", "."].concat();
        let senders: Vec<String> = sources()
            .into_iter()
            .filter(|(_, text)| text.contains(&domain))
            .map(|(path, _)| path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(senders, ["input.rs"]);
    }
}
