/// Global bounding rectangle of the macOS virtual external display (in CoreGraphics logical points).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DisplayRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// Physical pixel resolution of the controlled Linux display.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientResolution {
    pub width: u32,
    pub height: u32,
}

impl DisplayRect {
    /// Half-open containment `[x, x + width) × [y, y + height)` matching CoreGraphics display tiling.
    #[inline]
    pub fn contains(&self, px: f64, py: f64) -> bool {
        self.width > 0.0
            && self.height > 0.0
            && px >= self.x
            && px < self.x + self.width
            && py >= self.y
            && py < self.y + self.height
    }
}

/// Maps a global macOS cursor position `(px, py)` to absolute Linux display coordinates `[0..width-1, 0..height-1]`.
///
/// Returns `None` if `(px, py)` is outside the virtual external display (i.e., pointer is on a Mac display).
/// When the virtual display bounds match `client_res` 1:1 (default), each integer point maps 1:1 to the exact client pixel.
#[inline]
pub fn map_pointer_to_client(
    virtual_bounds: DisplayRect,
    client_res: ClientResolution,
    px: f64,
    py: f64,
) -> Option<(u32, u32)> {
    if client_res.width == 0 || client_res.height == 0 || !virtual_bounds.contains(px, py) {
        return None;
    }

    let rel_x = px - virtual_bounds.x;
    let rel_y = py - virtual_bounds.y;

    let scaled_x = (rel_x / virtual_bounds.width) * (client_res.width as f64);
    let scaled_y = (rel_y / virtual_bounds.height) * (client_res.height as f64);

    let cx = (scaled_x.floor().max(0.0) as u32).min(client_res.width - 1);
    let cy = (scaled_y.floor().max(0.0) as u32).min(client_res.height - 1);

    Some((cx, cy))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_1_to_1_mapping_right_of_macbook() {
        // MacBook is [0..1512, 0..982), Virtual Display is placed to the right at [1512..3432, 0..1080)
        let bounds = DisplayRect {
            x: 1512.0,
            y: 0.0,
            width: 1920.0,
            height: 1080.0,
        };
        let res = ClientResolution {
            width: 1920,
            height: 1080,
        };

        // On MacBook -> None
        assert_eq!(map_pointer_to_client(bounds, res, 0.0, 0.0), None);
        assert_eq!(map_pointer_to_client(bounds, res, 1511.996, 500.0), None);

        // First pixel inside virtual display -> (0, 0)
        assert_eq!(
            map_pointer_to_client(bounds, res, 1512.0, 0.0),
            Some((0, 0))
        );
        // Middle pixel -> (960, 540)
        assert_eq!(
            map_pointer_to_client(bounds, res, 1512.0 + 960.0, 540.0),
            Some((960, 540))
        );
        // Bottom-right corner pixel -> (1919, 1079)
        assert_eq!(
            map_pointer_to_client(bounds, res, 1512.0 + 1919.99, 1079.99),
            Some((1919, 1079))
        );
    }

    #[test]
    fn exact_1_to_1_mapping_left_or_above_macbook() {
        // Virtual display arranged to the LEFT of MacBook with vertical offset (-2560, -200)
        let bounds = DisplayRect {
            x: -2560.0,
            y: -200.0,
            width: 2560.0,
            height: 1440.0,
        };
        let res = ClientResolution {
            width: 2560,
            height: 1440,
        };

        // At x = 0.0 we are back on MacBook!
        assert_eq!(map_pointer_to_client(bounds, res, 0.0, 0.0), None);
        // Just left of MacBook border (-0.01) -> right edge of Linux monitor (2559)
        assert_eq!(
            map_pointer_to_client(bounds, res, -0.01, -200.0),
            Some((2559, 0))
        );
        // Far top-left of virtual display -> (0, 0)
        assert_eq!(
            map_pointer_to_client(bounds, res, -2560.0, -200.0),
            Some((0, 0))
        );
    }
}

