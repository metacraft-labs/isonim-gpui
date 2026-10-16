//! The window frame the renderer draws when the application is expected to
//! decorate its own window — client-side decorations, as on GNOME/Wayland.
//!
//! In that mode the platform window is larger than the application's content:
//! a transparent band of `FRAME_INSET` on every untiled side carries the
//! shadow, the 1px border and the outer half of the resize bands. GPUI is told
//! about the band (`Window::set_client_inset`) and sets the compositor's window
//! geometry to the content area (`gpui-pre-linux` `wayland/window.rs`,
//! `inset_by_tiling`), so what the compositor reports as the window IS the
//! application's content. The border is drawn inside the band, outside that
//! geometry, for the same reason.
//!
//! This module is the geometry only — no GPUI types — so it is tested without
//! a window. `gpui_app::frame_client_window` draws it.

use crate::window::{TILING_BOTTOM, TILING_LEFT, TILING_RIGHT, TILING_TOP};

/// The transparent band around an untiled side: shadow, border, resize reach.
pub const FRAME_INSET: f32 = 10.0;
/// The border, drawn on the content's edge inside the band.
pub const FRAME_BORDER: f32 = 1.0;
/// How far a resize band reaches on each side of the content's edge.
pub const RESIZE_REACH: f32 = 5.0;
/// How far a corner reaches along each of its two edges, into the content.
pub const RESIZE_CORNER: f32 = 12.0;

/// Which edge or corner a resize grabs (GPUI's `ResizeEdge`, without GPUI).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    Top,
    TopRight,
    Right,
    BottomRight,
    Bottom,
    BottomLeft,
    Left,
    TopLeft,
}

/// A rectangle in window coordinates (the platform window, band included).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Zone {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Zone {
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.w && y < self.y + self.h
    }
}

/// The frame of one window, for one frame of rendering.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameGeometry {
    /// The band on each side; 0 where the side is tiled, the window is
    /// maximized, or the compositor draws the decorations.
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    /// The platform window's size, band included (`Window::viewport_size`).
    pub viewport_w: f32,
    pub viewport_h: f32,
}

impl FrameGeometry {
    /// `client`: the application draws the decorations. `tiling`: the
    /// `window::TILING_*` bits. A maximized window has no frame at all,
    /// whatever its tiling reports.
    pub fn new(
        client: bool,
        tiling: u8,
        maximized: bool,
        viewport_w: f32,
        viewport_h: f32,
    ) -> Self {
        let side = |bit: u8| {
            if client && !maximized && tiling & bit == 0 {
                FRAME_INSET
            } else {
                0.0
            }
        };
        FrameGeometry {
            left: side(TILING_LEFT),
            top: side(TILING_TOP),
            right: side(TILING_RIGHT),
            bottom: side(TILING_BOTTOM),
            viewport_w,
            viewport_h,
        }
    }

    /// The inset to hand GPUI: it takes one value for every untiled side,
    /// and this frame uses the same band on each.
    pub fn client_inset(&self) -> f32 {
        self.left.max(self.top).max(self.right).max(self.bottom)
    }

    /// Whether there is any frame to draw.
    pub fn is_framed(&self) -> bool {
        self.client_inset() > 0.0
    }

    /// The content's origin in window coordinates — what pointer positions
    /// are translated by before they reach the application.
    pub fn content_origin(&self) -> (f32, f32) {
        (self.left, self.top)
    }

    /// The content's size: the viewport less the band on each framed side.
    pub fn content_size(&self) -> (f32, f32) {
        (
            (self.viewport_w - self.left - self.right).max(0.0),
            (self.viewport_h - self.top - self.bottom).max(0.0),
        )
    }

    /// The resize zones, corners first (they win where they overlap an
    /// edge). Only untiled sides resize; a corner needs both of its sides.
    pub fn resize_zones(&self) -> Vec<(Edge, Zone)> {
        if !self.is_framed() {
            return Vec::new();
        }
        let (cw, ch) = self.content_size();
        let (l, t) = (self.left, self.top);
        let (r, b) = (l + cw, t + ch);
        let (reach, corner) = (RESIZE_REACH, RESIZE_CORNER);
        let (on_l, on_t, on_r, on_b) = (
            self.left > 0.0,
            self.top > 0.0,
            self.right > 0.0,
            self.bottom > 0.0,
        );
        let mut zones = Vec::new();
        let sq = reach + corner;
        if on_t && on_l {
            zones.push((
                Edge::TopLeft,
                Zone {
                    x: l - reach,
                    y: t - reach,
                    w: sq,
                    h: sq,
                },
            ));
        }
        if on_t && on_r {
            zones.push((
                Edge::TopRight,
                Zone {
                    x: r - corner,
                    y: t - reach,
                    w: sq,
                    h: sq,
                },
            ));
        }
        if on_b && on_l {
            zones.push((
                Edge::BottomLeft,
                Zone {
                    x: l - reach,
                    y: b - corner,
                    w: sq,
                    h: sq,
                },
            ));
        }
        if on_b && on_r {
            zones.push((
                Edge::BottomRight,
                Zone {
                    x: r - corner,
                    y: b - corner,
                    w: sq,
                    h: sq,
                },
            ));
        }
        let span = 2.0 * reach;
        if on_t {
            zones.push((
                Edge::Top,
                Zone {
                    x: l,
                    y: t - reach,
                    w: cw,
                    h: span,
                },
            ));
        }
        if on_b {
            zones.push((
                Edge::Bottom,
                Zone {
                    x: l,
                    y: b - reach,
                    w: cw,
                    h: span,
                },
            ));
        }
        if on_l {
            zones.push((
                Edge::Left,
                Zone {
                    x: l - reach,
                    y: t,
                    w: span,
                    h: ch,
                },
            ));
        }
        if on_r {
            zones.push((
                Edge::Right,
                Zone {
                    x: r - reach,
                    y: t,
                    w: span,
                    h: ch,
                },
            ));
        }
        zones
    }

    /// The edge a press at window position (x, y) resizes, if any.
    pub fn resize_edge_at(&self, x: f32, y: f32) -> Option<Edge> {
        self.resize_zones()
            .into_iter()
            .find(|(_, z)| z.contains(x, y))
            .map(|(e, _)| e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: f32 = 820.0;
    const H: f32 = 620.0;

    #[test]
    fn server_decorations_have_no_frame() {
        let f = FrameGeometry::new(false, 0, false, W, H);
        assert!(!f.is_framed());
        assert_eq!(f.content_origin(), (0.0, 0.0));
        assert_eq!(f.content_size(), (W, H));
        assert!(f.resize_zones().is_empty());
    }

    #[test]
    fn an_untiled_client_window_is_framed_on_every_side() {
        let f = FrameGeometry::new(true, 0, false, W, H);
        assert_eq!(f.client_inset(), FRAME_INSET);
        assert_eq!(f.content_origin(), (FRAME_INSET, FRAME_INSET));
        assert_eq!(
            f.content_size(),
            (W - 2.0 * FRAME_INSET, H - 2.0 * FRAME_INSET)
        );
        assert_eq!(f.resize_zones().len(), 8);
    }

    #[test]
    fn a_maximized_window_has_no_frame_whatever_its_tiling_says() {
        let f = FrameGeometry::new(true, 0, true, W, H);
        assert!(!f.is_framed());
        assert_eq!(f.content_size(), (W, H));
    }

    #[test]
    fn a_tiled_side_has_no_band_and_no_resize() {
        // Tiled on the left (a half-screen snap): no band there, no left
        // edge, no left corners; the other three sides still resize.
        let f = FrameGeometry::new(true, TILING_LEFT, false, W, H);
        assert_eq!(f.left, 0.0);
        assert_eq!(f.content_origin(), (0.0, FRAME_INSET));
        let edges: Vec<Edge> = f.resize_zones().into_iter().map(|(e, _)| e).collect();
        assert!(!edges.contains(&Edge::Left));
        assert!(!edges.contains(&Edge::TopLeft));
        assert!(!edges.contains(&Edge::BottomLeft));
        assert!(edges.contains(&Edge::Right) && edges.contains(&Edge::TopRight));
        assert_eq!(f.resize_edge_at(1.0, H / 2.0), None);
    }

    #[test]
    fn every_edge_is_found_at_its_own_place_and_corners_win() {
        let f = FrameGeometry::new(true, 0, false, W, H);
        let (l, t) = f.content_origin();
        let (cw, ch) = f.content_size();
        let (r, b) = (l + cw, t + ch);
        let (mx, my) = (l + cw / 2.0, t + ch / 2.0);
        // Just outside the content (in the band) and just inside it.
        for d in [-3.0, 3.0] {
            assert_eq!(f.resize_edge_at(mx, t + d), Some(Edge::Top));
            assert_eq!(f.resize_edge_at(mx, b + d), Some(Edge::Bottom));
            assert_eq!(f.resize_edge_at(l + d, my), Some(Edge::Left));
            assert_eq!(f.resize_edge_at(r + d, my), Some(Edge::Right));
            assert_eq!(f.resize_edge_at(l + d, t + d), Some(Edge::TopLeft));
            assert_eq!(f.resize_edge_at(r + d, t + d), Some(Edge::TopRight));
            assert_eq!(f.resize_edge_at(l + d, b + d), Some(Edge::BottomLeft));
            assert_eq!(f.resize_edge_at(r + d, b + d), Some(Edge::BottomRight));
        }
        // Along an edge but within a corner's reach: the corner wins.
        assert_eq!(f.resize_edge_at(l + 8.0, t), Some(Edge::TopLeft));
        // The content's middle and the band's outer half resize nothing.
        assert_eq!(f.resize_edge_at(mx, my), None);
        assert_eq!(f.resize_edge_at(mx, 1.0), None);
    }
}
