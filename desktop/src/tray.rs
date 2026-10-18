// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! The tray icon's backup status: a red dot on the icon while a backup has
//! failed or is overdue, and a tooltip that says which.

use std::time::Duration;

use desktop_core::status::BackupStatusSource;
use tauri::{AppHandle, Manager, image::Image};

use crate::app::{Desktop, TRAY_ID};

/// How often the tray asks the server how the backups are doing.
const POLL_INTERVAL: Duration = Duration::from_mins(1);
/// The dot's colour (RGBA), the same red the web UI uses for a failure.
const DOT_COLOUR: [u8; 4] = [0xdc, 0x26, 0x26, 0xff];
/// The dot's radius, as a fraction of the icon's width.
const DOT_RADIUS: f32 = 0.22;

/// Keeps the tray icon's status current until the app quits.
pub async fn watch(app: AppHandle, source: BackupStatusSource) {
    let Some(normal) = app.default_window_icon().map(owned) else {
        tracing::warn!("the app has no icon, so the tray can't show backup status");
        return;
    };
    let attention = with_dot(&normal);
    let mut shown = None;
    while !app.state::<Desktop>().is_quitting() {
        match source.fetch().await {
            Ok(health) if shown != Some(health) => {
                let icon = if health.needs_attention() {
                    attention.clone()
                } else {
                    normal.clone()
                };
                show(&app, &icon, &health.tooltip());
                shown = Some(health);
            }
            Ok(_) => {}
            // The server may be restarting, or the app quitting: keep
            // showing the last known status rather than guessing.
            Err(e) => tracing::warn!(error = %e, "failed to read the backup status"),
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

fn show(app: &AppHandle, icon: &Image<'static>, tooltip: &str) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return;
    };
    let shown = tray
        .set_icon(Some(icon.clone()))
        .and_then(|()| tray.set_tooltip(Some(tooltip)));
    if let Err(e) = shown {
        tracing::warn!(error = %e, "failed to update the tray icon");
    }
}

/// A copy of `icon` that doesn't borrow from the app.
fn owned(icon: &Image<'_>) -> Image<'static> {
    Image::new_owned(icon.rgba().to_vec(), icon.width(), icon.height())
}

/// `icon` with a red dot in its bottom-right corner.
fn with_dot(icon: &Image<'_>) -> Image<'static> {
    let mut rgba = icon.rgba().to_vec();
    paint_dot(&mut rgba, icon.width());
    Image::new_owned(rgba, icon.width(), icon.height())
}

/// Paints a filled circle into the bottom-right corner of an RGBA image
/// `width` pixels wide, its edge touching the image's right and bottom
/// edges.
fn paint_dot(rgba: &mut [u8], width: u32) {
    let Ok(width_px) = u16::try_from(width) else {
        return;
    };
    let Some(row_len) = usize::from(width_px).checked_mul(4) else {
        return;
    };
    if row_len == 0 {
        return;
    }
    let rows = rgba.len().checked_div(row_len).unwrap_or(0);
    let Ok(rows) = u16::try_from(rows) else {
        return;
    };
    let radius = f32::from(width_px) * DOT_RADIUS;
    let centre_x = f32::from(width_px) - radius;
    let centre_y = f32::from(rows) - radius;
    for (y, row) in (0u16..).zip(rgba.chunks_exact_mut(row_len)) {
        for (x, pixel) in (0u16..).zip(row.as_chunks_mut::<4>().0) {
            // Each pixel is sampled at its centre.
            let dx = f32::from(x) + 0.5 - centre_x;
            let dy = f32::from(y) + 0.5 - centre_y;
            if dx * dx + dy * dy <= radius * radius {
                *pixel = DOT_COLOUR;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLEAR: [u8; 4] = [0, 0, 0, 0];
    const SIDE: u32 = 32;
    const ROW_BYTES: usize = 32 * 4;

    fn pixel(rgba: &[u8], x: usize, y: usize) -> [u8; 4] {
        let row = rgba.as_chunks::<ROW_BYTES>().0.get(y).unwrap();
        *row.as_chunks::<4>().0.get(x).unwrap()
    }

    #[test]
    fn the_dot_sits_in_the_bottom_right_corner() {
        let mut rgba = vec![0; 32 * 32 * 4];

        paint_dot(&mut rgba, SIDE);

        // The dot's centre is 7 pixels in from each edge (0.22 * 32).
        assert_eq!(pixel(&rgba, 25, 25), DOT_COLOUR);
        assert_eq!(pixel(&rgba, 31, 25), DOT_COLOUR);
        assert_eq!(pixel(&rgba, 0, 0), CLEAR);
        assert_eq!(pixel(&rgba, 31, 0), CLEAR);
        assert_eq!(pixel(&rgba, 0, 31), CLEAR);
        // The corner itself lies outside the circle.
        assert_eq!(pixel(&rgba, 31, 31), CLEAR);
    }

    #[test]
    fn the_rest_of_the_icon_is_untouched() {
        let mut rgba = vec![7; 32 * 32 * 4];

        paint_dot(&mut rgba, SIDE);

        assert_eq!(pixel(&rgba, 4, 4), [7; 4]);
        assert_eq!(pixel(&rgba, 16, 30), [7; 4]);
    }

    #[test]
    fn a_degenerate_image_is_left_alone() {
        let mut empty: Vec<u8> = Vec::new();
        paint_dot(&mut empty, 0);
        assert_eq!(empty, Vec::<u8>::new());
    }
}
