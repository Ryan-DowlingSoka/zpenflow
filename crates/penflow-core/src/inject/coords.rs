//! Input-area → output-area coordinate transform.
//!
//! Replaces the predecessor's naive `left + norm * width` with a 2D affine
//! transform — design.md §6.6 says "Implemented as a single Matrix3x2 so
//! future 'rotate the tablet 90°' is one parameter". Using a hand-rolled
//! 6-float affine instead of pulling in `nalgebra` for one matrix, since
//! v1.0 doesn't ship rotation; the math is identical and the form swaps in
//! cleanly if/when rotation lands.
//!
//! Coordinate convention:
//!   - Pen samples arrive normalized to [0, 1] × [0, 1] over the **input
//!     area** (the Android tablet panel, after dead-zone trimming).
//!   - Output is virtual-screen pixels (after `SetProcessDpiAwarenessContext`
//!     so they're physical pixels, not DIPs — gate-2 finding §4.4b).

#[derive(Clone, Copy, Debug)]
pub struct AffineTransform {
    // 2D affine, row-major:
    //   [ a  c  e ]   [ x ]
    //   [ b  d  f ] * [ y ]
    //   [ 0  0  1 ]   [ 1 ]
    a: f32,
    b: f32,
    c: f32,
    d: f32,
    e: f32,
    f: f32,
}

impl AffineTransform {
    pub fn identity() -> Self {
        Self {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            e: 0.0,
            f: 0.0,
        }
    }

    /// Build a transform that maps `[0, 1] × [0, 1]` (raw normalized pen
    /// coordinates) onto the output rectangle, with optional rotation in
    /// 90-degree steps applied to the input first.
    ///
    /// `rotation_deg` is one of 0 / 90 / 180 / 270; other values fall back
    /// to 0 (we don't do arbitrary rotation in v1.0 — the tablet ships in
    /// landscape and Krita's portrait path goes through Krita rotation, not
    /// ours).
    pub fn from_normalized_to_rect(
        output_left: i32,
        output_top: i32,
        output_w: u32,
        output_h: u32,
        rotation_deg: u32,
    ) -> Self {
        let ow = output_w as f32;
        let oh = output_h as f32;
        let ol = output_left as f32;
        let ot = output_top as f32;
        match rotation_deg % 360 {
            0 => Self {
                a: ow,
                b: 0.0,
                c: 0.0,
                d: oh,
                e: ol,
                f: ot,
            },
            90 => Self {
                // (x, y) → (oh - y * oh, x * ow) then translate. Equivalent
                // affine: x' = -oh * y + ol + ow ;  y' = ow * x + ot
                a: 0.0,
                b: ow,
                c: -ow,
                d: 0.0,
                e: ol + ow,
                f: ot,
            },
            180 => Self {
                a: -ow,
                b: 0.0,
                c: 0.0,
                d: -oh,
                e: ol + ow,
                f: ot + oh,
            },
            270 => Self {
                a: 0.0,
                b: -oh,
                c: ow,
                d: 0.0,
                e: ol,
                f: ot + oh,
            },
            _ => Self::from_normalized_to_rect(output_left, output_top, output_w, output_h, 0),
        }
    }

    /// Return a copy whose output is shifted by `(dx, dy)` output pixels.
    /// The shift is applied after rotation, so it moves the result in
    /// desktop space regardless of tablet orientation — used for the
    /// pen-tip parallax offset.
    pub fn translated(&self, dx: f32, dy: f32) -> Self {
        Self {
            e: self.e + dx,
            f: self.f + dy,
            ..*self
        }
    }

    /// Apply the transform to a single point.
    pub fn map(&self, x: f32, y: f32) -> (f32, f32) {
        (
            self.a * x + self.c * y + self.e,
            self.b * x + self.d * y + self.f,
        )
    }

    /// Apply and snap to integer pixels (the Win32 / WinRT injection APIs
    /// take `i32` coordinates).
    pub fn map_to_pixel(&self, x: f32, y: f32) -> (i32, i32) {
        let (fx, fy) = self.map(x, y);
        (fx.round() as i32, fy.round() as i32)
    }

    /// Map normalized pen coords `[0,1]²` to VMulti's logical units
    /// `[0, 32767]²`, scaled across the target rectangle
    /// `(target_left, target_top, target_w_px, target_h_px)`.
    ///
    /// VMulti's HID descriptor declares `logical_min/max = 0..32767` per
    /// axis. The receiver-side mapping from those logical units onto
    /// screen pixels happens inside the Windows kernel, using the
    /// digitizer's physical-axis declaration plus the monitor it's
    /// associated with. For a digitizer that spans the full virtual
    /// screen, callers pass the virtual-screen bounding box
    /// (`SM_XVIRTUALSCREEN`, `SM_YVIRTUALSCREEN`, `SM_CXVIRTUALSCREEN`,
    /// `SM_CYVIRTUALSCREEN`) here.
    ///
    /// The affine outputs primary-relative desktop pixels, which go
    /// negative for monitors left of / above the primary. Logical 0 is
    /// the bounding box's top-left corner, not the primary's, so the
    /// target origin is subtracted before scaling — the same
    /// primary-relative → bbox-relative shift `win_ink` applies on the
    /// `InjectSyntheticPointerInput` path. Without it, a layout with a
    /// monitor left of the primary lands the pen one monitor-width away
    /// from the tip.
    pub fn map_to_vmulti(
        &self,
        x: f32,
        y: f32,
        target_left: i32,
        target_top: i32,
        target_w_px: u32,
        target_h_px: u32,
    ) -> (u16, u16) {
        let (fx, fy) = self.map(x, y);
        let rx = fx - target_left as f32;
        let ry = fy - target_top as f32;
        let tw = target_w_px.max(1) as f32;
        let th = target_h_px.max(1) as f32;
        let ux = ((rx / tw) * 32767.0).clamp(0.0, 32767.0).round() as u16;
        let uy = ((ry / th) * 32767.0).clamp(0.0, 32767.0).round() as u16;
        (ux, uy)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: (f32, f32), b: (f32, f32), eps: f32) -> bool {
        (a.0 - b.0).abs() < eps && (a.1 - b.1).abs() < eps
    }

    #[test]
    fn identity_is_passthrough() {
        let t = AffineTransform::identity();
        assert!(approx(t.map(0.5, 0.7), (0.5, 0.7), 1e-6));
    }

    #[test]
    fn maps_corners_for_rect() {
        let t = AffineTransform::from_normalized_to_rect(100, 200, 1920, 1080, 0);
        assert_eq!(t.map_to_pixel(0.0, 0.0), (100, 200));
        assert_eq!(t.map_to_pixel(1.0, 0.0), (2020, 200));
        assert_eq!(t.map_to_pixel(0.0, 1.0), (100, 1280));
        assert_eq!(t.map_to_pixel(1.0, 1.0), (2020, 1280));
        assert_eq!(t.map_to_pixel(0.5, 0.5), (1060, 740));
    }

    #[test]
    fn map_to_vmulti_spans_full_logical_range() {
        // VDD at origin, 3840x2160; tablet norm [0,1] → VDD pixel [0..3840, 0..2160]
        // → VMulti logical [0..32767].
        let t = AffineTransform::from_normalized_to_rect(0, 0, 3840, 2160, 0);
        assert_eq!(t.map_to_vmulti(0.0, 0.0, 0, 0, 3840, 2160), (0, 0));
        assert_eq!(t.map_to_vmulti(1.0, 1.0, 0, 0, 3840, 2160), (32767, 32767));
        let (mx, my) = t.map_to_vmulti(0.5, 0.5, 0, 0, 3840, 2160);
        assert!(mx.abs_diff(16383) <= 1 && my.abs_diff(16383) <= 1);
    }

    #[test]
    fn map_to_vmulti_offset_rect_lands_proportionally() {
        // VDD at (1920, 0), 1920x1080; on a virtual screen 3840x1080, this
        // covers the right half. Tablet (0,0) → VDD top-left → virtual
        // pixel (1920, 0) → VMulti logical (16383, 0).
        let t = AffineTransform::from_normalized_to_rect(1920, 0, 1920, 1080, 0);
        let (mx, my) = t.map_to_vmulti(0.0, 0.0, 0, 0, 3840, 1080);
        assert!(mx.abs_diff(16383) <= 1, "got {mx}");
        assert_eq!(my, 0);
        // Tablet (1,1) → VDD bottom-right pixel (3840, 1080) → VMulti
        // logical (32767, 32767).
        let (mx, my) = t.map_to_vmulti(1.0, 1.0, 0, 0, 3840, 1080);
        assert_eq!(mx, 32767);
        assert_eq!(my, 32767);
    }

    #[test]
    fn map_to_vmulti_subtracts_negative_virtual_origin() {
        // Three 3840x2160 monitors: left at x=-3840, primary at 0, right
        // at 3840. Virtual screen bbox = (-3840, 0, 11520, 2160).
        let (vl, vt, vw, vh) = (-3840, 0, 11520, 2160);

        // Capturing the primary: its left edge is one third of the way
        // across the bbox, its right edge two thirds.
        let primary = AffineTransform::from_normalized_to_rect(0, 0, 3840, 2160, 0);
        let (mx, my) = primary.map_to_vmulti(0.0, 0.0, vl, vt, vw, vh);
        assert!(mx.abs_diff(10922) <= 1, "got {mx}");
        assert_eq!(my, 0);
        let (mx, _) = primary.map_to_vmulti(1.0, 1.0, vl, vt, vw, vh);
        assert!(mx.abs_diff(21845) <= 1, "got {mx}");

        // Capturing the left monitor: spans the first third of the bbox
        // instead of collapsing onto logical 0.
        let left = AffineTransform::from_normalized_to_rect(-3840, 0, 3840, 2160, 0);
        let (mx, _) = left.map_to_vmulti(0.0, 0.5, vl, vt, vw, vh);
        assert_eq!(mx, 0);
        let (mx, _) = left.map_to_vmulti(0.5, 0.5, vl, vt, vw, vh);
        assert!(mx.abs_diff(5461) <= 1, "got {mx}");
        let (mx, _) = left.map_to_vmulti(1.0, 0.5, vl, vt, vw, vh);
        assert!(mx.abs_diff(10922) <= 1, "got {mx}");
    }

    #[test]
    fn map_to_vmulti_subtracts_negative_vertical_origin() {
        // 4K monitor taller than a 1080p primary to its left, top at -1080.
        // Virtual bbox = (-3840, -1080, 5760, 2160). Primary's top edge
        // sits halfway down the bbox.
        let primary = AffineTransform::from_normalized_to_rect(0, 0, 1920, 1080, 0);
        let (mx, my) = primary.map_to_vmulti(0.0, 0.0, -3840, -1080, 5760, 2160);
        assert!(mx.abs_diff(21845) <= 1, "got {mx}");
        assert!(my.abs_diff(16383) <= 1, "got {my}");
    }

    #[test]
    fn translated_shifts_output_in_desktop_space() {
        let t =
            AffineTransform::from_normalized_to_rect(100, 200, 1920, 1080, 0).translated(3.0, -4.0);
        assert_eq!(t.map_to_pixel(0.0, 0.0), (103, 196));
        assert_eq!(t.map_to_pixel(1.0, 1.0), (2023, 1276));

        // Rotated: the shift still lands in desktop x/y, not tablet x/y.
        let r = AffineTransform::from_normalized_to_rect(0, 0, 100, 200, 90);
        let rt = r.translated(5.0, 7.0);
        let (bx, by) = r.map_to_pixel(0.3, 0.6);
        assert_eq!(rt.map_to_pixel(0.3, 0.6), (bx + 5, by + 7));
    }

    #[test]
    fn rotates_90_landscape_to_portrait() {
        // Input area [0,1]² rotated 90° onto a 100×200 output rect at origin.
        // Top-left of input (0,0) should land at top-right of output (100,0).
        let t = AffineTransform::from_normalized_to_rect(0, 0, 100, 200, 90);
        assert_eq!(t.map_to_pixel(0.0, 0.0), (100, 0));
        assert_eq!(t.map_to_pixel(1.0, 0.0), (100, 100));
        assert_eq!(t.map_to_pixel(0.0, 1.0), (0, 0));
        assert_eq!(t.map_to_pixel(1.0, 1.0), (0, 100));
    }
}
