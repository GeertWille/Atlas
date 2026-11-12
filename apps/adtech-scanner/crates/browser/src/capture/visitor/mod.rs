//! The scripted visitor: what it does on a page and which link it follows.
//! Everything in here is planning and policy, decided from a seed and from
//! facts about the page. Nothing in here talks to the browser.

pub(crate) mod links;
pub(crate) mod motion;
pub(crate) mod plan;
pub(crate) mod rng;
pub(crate) mod site;

use serde::Deserialize;

/// A position in the viewport, in CSS pixels.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
pub(crate) struct Point {
    pub x: f64,
    pub y: f64,
}

impl Point {
    pub(crate) fn distance_to(self, other: Point) -> f64 {
        (other.x - self.x).hypot(other.y - self.y)
    }

    /// The same point seen from a frame whose origin is `by` further out.
    pub(crate) fn offset_by(self, by: Point) -> Point {
        Point {
            x: self.x + by.x,
            y: self.y + by.y,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
pub(crate) struct Size {
    pub width: f64,
    pub height: f64,
}

impl Size {
    /// The nearest position that is inside, a pixel away from the edges.
    pub(crate) fn clamp(self, point: Point) -> Point {
        Point {
            x: point.x.clamp(1.0, (self.width - 1.0).max(1.0)),
            y: point.y.clamp(1.0, (self.height - 1.0).max(1.0)),
        }
    }
}

/// A box relative to the viewport, in CSS pixels.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
pub(crate) struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect {
    pub(crate) fn center(self) -> Point {
        Point {
            x: self.x + self.width / 2.0,
            y: self.y + self.height / 2.0,
        }
    }

    /// The position `across` and `down` the box, each as a 0-1 share.
    pub(crate) fn point_at(self, across: f64, down: f64) -> Point {
        Point {
            x: self.x + self.width * across,
            y: self.y + self.height * down,
        }
    }

    pub(crate) fn is_inside(self, viewport: Size) -> bool {
        self.x >= 0.0
            && self.y >= 0.0
            && self.x + self.width <= viewport.width
            && self.y + self.height <= viewport.height
    }
}

/// What the page says about itself when the visitor looks at it
/// (`injected/links.js`).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PageFacts {
    pub viewport: Size,
    pub scroll_top: f64,
    pub page_height: f64,
    /// Things to look at that are in view.
    pub targets: Vec<ReadingTarget>,
    /// Empty unless links were asked for.
    pub links: Vec<links::LinkFacts>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub(crate) struct ReadingTarget {
    /// "paragraph", "heading" or "image".
    pub kind: String,
    pub rect: Rect,
}

impl PageFacts {
    /// The page as the planner sees it, with the pointer at `pointer`.
    pub(crate) fn state(&self, pointer: Point) -> plan::PageState {
        plan::PageState {
            viewport: self.viewport,
            scroll_top: self.scroll_top,
            page_height: self.page_height,
            pointer: self.viewport.clamp(pointer),
            reading_targets: self.targets.iter().map(|target| target.rect).collect(),
        }
    }

    /// 0-1 share of the scrollable height that is scrolled.
    pub(crate) fn scroll_depth(&self) -> f64 {
        let scrollable = (self.page_height - self.viewport.height).max(1.0);
        (self.scroll_top / scrollable).clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_facts_are_read_from_what_the_page_script_returns() {
        let facts: PageFacts = serde_json::from_str(
            r#"{"viewport":{"width":1440,"height":900},"scrollTop":1275,"pageHeight":6000,
                "targets":[{"kind":"heading","rect":{"x":200,"y":80,"width":640,"height":48}}],
                "links":[]}"#,
        )
        .unwrap();
        assert_eq!(facts.scroll_depth(), 0.25);
        assert_eq!(facts.targets[0].kind, "heading");
        let state = facts.state(Point {
            x: 5_000.0,
            y: -3.0,
        });
        assert_eq!(state.pointer, Point { x: 1439.0, y: 1.0 });
        assert_eq!(state.reading_targets.len(), 1);
        assert_eq!(state.scroll_top, 1_275.0);
    }

    #[test]
    fn a_page_that_cannot_scroll_has_depth_zero() {
        let facts = PageFacts {
            viewport: Size {
                width: 800.0,
                height: 600.0,
            },
            scroll_top: 0.0,
            page_height: 600.0,
            targets: Vec::new(),
            links: Vec::new(),
        };
        assert_eq!(facts.scroll_depth(), 0.0);
    }
}
