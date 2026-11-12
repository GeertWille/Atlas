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
