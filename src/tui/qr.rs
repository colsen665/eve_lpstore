//! QR codes for the TUI. The modules come from the `qrcode` crate, with a quiet zone around them.

use qrcode::{Color, QrCode};

/// Light modules around the code. The QR code specification asks for four.
const QUIET_ZONE: usize = 4;

/// The modules of a QR code for `data`, row by row, with `true` for dark modules. The quiet zone
/// is included. The row count is even, so the rows can be drawn two at a time.
pub fn modules(data: &str) -> Vec<Vec<bool>> {
    let code = QrCode::new(data.as_bytes()).expect("the QR code data fits in a code");
    let width = code.width();
    let colors = code.to_colors();
    let total = width + 2 * QUIET_ZONE;
    let rows = total + total % 2;
    let mut grid = vec![vec![false; total]; rows];
    for y in 0..width {
        for x in 0..width {
            grid[y + QUIET_ZONE][x + QUIET_ZONE] = colors[y * width + x] == Color::Dark;
        }
    }
    grid
}

#[cfg(test)]
mod tests {
    use super::*;

    const ADDRESS: &str = "bc1qyc55ddky2ehdnty2c48nvxxugjw2xdpxk956x7";

    #[test]
    fn the_address_fits_a_version_three_code_with_a_quiet_zone() {
        let grid = modules(ADDRESS);
        assert_eq!(grid[0].len(), 29 + 2 * QUIET_ZONE);
        assert_eq!(grid.len() % 2, 0);
    }

    #[test]
    fn the_quiet_zone_is_light() {
        let grid = modules(ADDRESS);
        let last = grid[0].len() - 1;
        assert!(
            grid[..QUIET_ZONE]
                .iter()
                .all(|row| row.iter().all(|&dark| !dark))
        );
        assert!(
            grid.iter()
                .all(|row| !row[..QUIET_ZONE].iter().any(|&dark| dark))
        );
        assert!(
            grid.iter()
                .all(|row| !row[last - QUIET_ZONE + 1..].iter().any(|&dark| dark))
        );
    }

    #[test]
    fn the_finder_pattern_sits_in_the_top_left_corner() {
        let grid = modules("https://example.org");
        let (x, y) = (QUIET_ZONE, QUIET_ZONE);
        assert!(grid[y][x], "the outer ring of the finder pattern is dark");
        assert!(!grid[y + 1][x + 1], "the ring inside it is light");
        assert!(
            grid[y + 3][x + 3],
            "the centre of the finder pattern is dark"
        );
    }
}
