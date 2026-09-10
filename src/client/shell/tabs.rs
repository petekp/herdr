use super::*;

const TAB_SCROLL_BUTTON_WIDTH: u16 = 3;
const MIN_TAB_STRIP_WIDTH: u16 =
    MIN_TAB_WIDTH + NEW_TAB_WIDTH + TAB_SCROLL_BUTTON_WIDTH.saturating_mul(2);

pub(crate) fn render_tab_bar(
    buffer: &mut Buffer,
    area: Rect,
    snapshot: &ClientShellSnapshot,
    config: &ClientShellConfig,
    tab_scroll: &mut usize,
    reveal_focused_tab: &mut bool,
    tab_drag_insert_index: Option<usize>,
    tab_swipe: Option<&super::super::tab_swipe::TabSwipe>,
    hits: &mut ShellHitMap,
) {
    let palette = &config.palette;
    // While a swipe is in flight the filled tab follows the gesture, not the
    // snapshot, so a focus change mid-animation does not jump.
    let swipe_anchor = tab_swipe.and_then(|swipe| {
        if swipe.progress >= 1.0 {
            swipe.target.as_ref().map(|target| target.tab_id.as_str())
        } else {
            Some(swipe.origin_tab_id.as_str())
        }
    });
    let swipe_overlay = tab_swipe.and_then(|swipe| {
        (swipe.progress > 0.0 && swipe.progress < 1.0).then_some((
            swipe.origin_tab_id.as_str(),
            swipe.target.as_ref()?.tab_id.as_str(),
        ))
    });
    let swipe_sliding = swipe_overlay.is_some();
    let mut swipe_origin: Option<(Rect, Style)> = None;
    let mut swipe_target: Option<Rect> = None;
    buffer.set_style(area, Style::default().bg(palette.panel_bg));
    let tabs = snapshot
        .tabs
        .iter()
        .filter(|tab| Some(tab.workspace_id.as_str()) == snapshot.focused_workspace_id.as_deref())
        .collect::<Vec<_>>();
    let desired_widths = tabs
        .iter()
        .map(|tab| {
            let label = tab_label(tab);
            display_width(&label).saturating_add(4).max(MIN_TAB_WIDTH)
        })
        .collect::<Vec<_>>();
    let content = tab_bar_content_area(snapshot, area);
    let mouse_chrome = config.mouse_capture;
    let new_tab_width = if mouse_chrome { NEW_TAB_WIDTH } else { 0 };
    let desired_total = desired_widths
        .iter()
        .copied()
        .fold(0_u16, u16::saturating_add)
        .saturating_add(tabs.len().saturating_sub(1).min(u16::MAX as usize) as u16)
        .saturating_add(new_tab_width);
    let overflow =
        desired_total > content.width && (!mouse_chrome || content.width >= MIN_TAB_STRIP_WIDTH);
    let available = if overflow && mouse_chrome {
        content
            .width
            .saturating_sub(NEW_TAB_WIDTH)
            .saturating_sub(TAB_SCROLL_BUTTON_WIDTH.saturating_mul(2))
    } else {
        content.width.saturating_sub(new_tab_width)
    };
    let max_scroll = max_tab_scroll(&desired_widths, available);
    let focused_index = tabs
        .iter()
        .position(|tab| swipe_anchor.map_or(tab.focused, |anchor| anchor == tab.tab_id));
    if !overflow {
        *tab_scroll = 0;
    } else if *reveal_focused_tab && !swipe_sliding {
        if let Some(focused) = focused_index {
            *tab_scroll = centered_tab_scroll(focused, &desired_widths, available).min(max_scroll);
        }
    } else {
        *tab_scroll = (*tab_scroll).min(max_scroll);
    }
    // A reveal requested while the fill is sliding waits until it lands, so
    // the strip does not scroll under the animation.
    if !swipe_sliding {
        *reveal_focused_tab = false;
    }

    let mut x = content.x;
    let tab_right = if overflow && mouse_chrome {
        hits.tab_scroll_left = Rect::new(
            content.x,
            content.y,
            TAB_SCROLL_BUTTON_WIDTH.min(content.width),
            1,
        );
        put_text(
            buffer,
            hits.tab_scroll_left.x,
            content.y,
            hits.tab_scroll_left.width,
            " < ",
            Style::default()
                .fg(if *tab_scroll > 0 {
                    palette.overlay1
                } else {
                    palette.overlay0
                })
                .bg(palette.surface0),
        );
        x = hits.tab_scroll_left.right();
        content
            .right()
            .saturating_sub(NEW_TAB_WIDTH + TAB_SCROLL_BUTTON_WIDTH)
    } else {
        content.right().saturating_sub(new_tab_width)
    };

    let mut first_visible = None;
    let mut last_visible = None;
    for (index, tab) in tabs.iter().enumerate().skip(*tab_scroll) {
        let name = tab_label(tab);
        let desired = desired_widths[index];
        let remaining = tab_right.saturating_sub(x);
        let width = desired.min(remaining);
        if width == 0 {
            break;
        }
        let rect = Rect::new(x, area.y, width, 1);
        let focused = focused_index == Some(index);
        let unfocused_style = if tab.custom_label {
            Style::default().fg(palette.overlay1).bg(palette.surface0)
        } else {
            Style::default()
                .fg(palette.overlay0)
                .bg(palette.surface0)
                .add_modifier(Modifier::DIM)
        };
        let style = if focused {
            let base = Style::default()
                .fg(panel_contrast_fg(palette))
                .bg(palette.accent);
            if tab.custom_label {
                base.add_modifier(Modifier::BOLD)
            } else {
                base
            }
        } else {
            unfocused_style
        };
        if let Some((origin_id, target_id)) = swipe_overlay {
            if tab.tab_id == origin_id {
                swipe_origin = Some((rect, unfocused_style));
            } else if tab.tab_id == target_id {
                swipe_target = Some(rect);
            }
        }
        let padding = width.saturating_sub(display_width(&name));
        let left = padding / 2;
        let text = format!(
            "{empty:left$}{name}{empty:right_padding$}",
            empty = "",
            left = left as usize,
            right_padding = padding.saturating_sub(left) as usize,
        );
        put_text(buffer, rect.x, rect.y, rect.width, &text, style);
        hits.tabs.push((rect, tab.tab_id.clone()));
        first_visible.get_or_insert(index);
        last_visible = Some(index);
        x = x.saturating_add(width + 1);
        if width < desired {
            break;
        }
    }

    if let (Some(swipe), Some((origin, origin_style))) = (tab_swipe, swipe_origin) {
        let wraps = swipe.target.as_ref().is_some_and(|target| target.wraps);
        match swipe_target {
            Some(target) if !wraps => render_tab_swipe_fill(
                buffer,
                palette,
                origin,
                origin_style,
                target,
                swipe.progress,
            ),
            // Wrapping around the strip, or a neighbor scrolled out of view:
            // the fill leaves the origin through the strip's edge.
            target => render_tab_swipe_drain(
                buffer,
                palette,
                origin,
                origin_style,
                target,
                swipe.direction(),
                swipe.progress,
            ),
        }
    }

    if overflow && mouse_chrome {
        hits.tab_scroll_right = Rect::new(tab_right, area.y, TAB_SCROLL_BUTTON_WIDTH, 1);
        let can_scroll_right = last_visible.is_some_and(|index| index + 1 < tabs.len());
        put_text(
            buffer,
            hits.tab_scroll_right.x,
            area.y,
            hits.tab_scroll_right.width,
            " > ",
            Style::default()
                .fg(if can_scroll_right {
                    palette.overlay1
                } else {
                    palette.overlay0
                })
                .bg(palette.surface0),
        );
        hits.new_tab = Rect::new(
            hits.tab_scroll_right.right(),
            area.y,
            content
                .right()
                .saturating_sub(hits.tab_scroll_right.right())
                .min(NEW_TAB_WIDTH),
            1,
        );
    } else if mouse_chrome {
        hits.new_tab = Rect::new(
            x.min(content.right()),
            area.y,
            content.right().saturating_sub(x).min(NEW_TAB_WIDTH),
            1,
        );
    }
    if mouse_chrome {
        put_text(
            buffer,
            hits.new_tab.x,
            area.y,
            hits.new_tab.width,
            " + ",
            Style::default().fg(palette.overlay1).bg(palette.panel_bg),
        );
    }

    if first_visible.is_some_and(|index| index > 0) {
        let ellipsis_x = if hits.tab_scroll_left.width > 0 {
            hits.tab_scroll_left.right()
        } else {
            content.x
        };
        put_text(
            buffer,
            ellipsis_x,
            area.y,
            u16::from(ellipsis_x < content.right()),
            "…",
            Style::default().fg(palette.overlay0),
        );
    }
    if last_visible.is_some_and(|index| index + 1 < tabs.len()) {
        let ellipsis_x = if hits.tab_scroll_right.width > 0 {
            hits.tab_scroll_right.x.saturating_sub(1)
        } else {
            content.right().saturating_sub(1)
        };
        put_text(
            buffer,
            ellipsis_x,
            area.y,
            u16::from(ellipsis_x >= content.x && ellipsis_x < content.right()),
            "…",
            Style::default().fg(palette.overlay0),
        );
    }

    if let Some(insert_index) = tab_drag_insert_index {
        if let Some(indicator_x) = tab_drop_indicator_x(hits, &tabs, insert_index) {
            put_text(
                buffer,
                indicator_x.min(content.right().saturating_sub(1)),
                area.y,
                1,
                "│",
                Style::default().fg(palette.accent),
            );
        }
    }
    render_tab_bar_status(buffer, area, snapshot, palette);
}

/// Left-aligned partial blocks, indexed by eighths filled.
const SWIPE_EDGE_BLOCKS: [&str; 8] = ["", "▏", "▎", "▍", "▌", "▋", "▊", "▉"];

/// Draws the focused-tab fill part way between the origin and target tabs.
fn render_tab_swipe_fill(
    buffer: &mut Buffer,
    palette: &Palette,
    origin: Rect,
    origin_style: Style,
    target: Rect,
    progress: f32,
) {
    buffer.set_style(origin, origin_style.remove_modifier(Modifier::BOLD));
    let lerp = |from: u16, to: u16| f32::from(from) + (f32::from(to) - f32::from(from)) * progress;
    paint_fill_span(
        buffer,
        palette,
        origin.y,
        lerp(origin.x, target.x),
        lerp(origin.right(), target.right()),
    );
}

/// Draws a fill leaving the origin through the strip's edge in the swipe
/// direction while the same share grows into the target from its far side.
/// Used when the swipe wraps around the strip and when the neighbor is
/// scrolled out of view; without a visible target only the draining half shows.
fn render_tab_swipe_drain(
    buffer: &mut Buffer,
    palette: &Palette,
    origin: Rect,
    origin_style: Style,
    target: Option<Rect>,
    direction: i32,
    progress: f32,
) {
    buffer.set_style(origin, origin_style.remove_modifier(Modifier::BOLD));
    let remaining = f32::from(origin.width) * (1.0 - progress);
    let entered = target.map_or(0.0, |target| f32::from(target.width) * progress);
    let y = origin.y;
    if direction < 0 {
        paint_fill_span(
            buffer,
            palette,
            y,
            f32::from(origin.x),
            f32::from(origin.x) + remaining,
        );
        if let Some(target) = target {
            let right = f32::from(target.right());
            paint_fill_span(buffer, palette, y, right - entered, right);
        }
    } else {
        let right = f32::from(origin.right());
        paint_fill_span(buffer, palette, y, right - remaining, right);
        if let Some(target) = target {
            let left = f32::from(target.x);
            paint_fill_span(buffer, palette, y, left, left + entered);
        }
    }
}

/// Style of a cell fully inside the moving fill.
fn filled_style(palette: &Palette) -> Style {
    Style::default()
        .fg(panel_contrast_fg(palette))
        .bg(palette.accent)
        .remove_modifier(Modifier::DIM)
}

/// Style for a partial-block edge cell. The tab's own dim and bold attributes
/// must not survive here: dim darkens the block glyph, which reads as a darker
/// sliver of fill at the moving edge.
fn edge_style(fg: ratatui::style::Color, bg: ratatui::style::Color) -> Style {
    Style::default()
        .fg(fg)
        .bg(bg)
        .remove_modifier(Modifier::DIM | Modifier::BOLD)
}

/// Paints the focused style over the fractional cell span `start..end` on
/// one row. Whole cells keep their text. The two edge cells use partial
/// block glyphs so the fill moves in eighths of a cell.
fn paint_fill_span(buffer: &mut Buffer, palette: &Palette, y: u16, start: f32, end: f32) {
    if end <= start || end.floor() == start.floor() {
        return;
    }
    let mut x = start.ceil();
    while x < end.floor() {
        if let Some(cell) = buffer.cell_mut((x as u16, y)) {
            cell.set_style(filled_style(palette));
        }
        x += 1.0;
    }
    let eighths = |fraction: f32| (fraction * 8.0).round() as usize;
    // Leading edge: the accent covers the right part of the cell.
    let leading = (start.floor() as u16, y);
    paint_split_cell(
        buffer,
        palette,
        leading,
        8 - eighths(start - start.floor()),
        false,
    );
    // Trailing edge: the accent covers the left part of the cell.
    let trailing = (end.floor() as u16, y);
    paint_split_cell(buffer, palette, trailing, eighths(end - end.floor()), true);
}

/// Paints an edge cell with `accent_eighths` of its width in the focused
/// style, on the left when `accent_left` and on the right otherwise. A cell
/// that holds or continues a wide glyph cannot take a block character, so it
/// takes the focused style only when the accent covers at least half of it.
fn paint_split_cell(
    buffer: &mut Buffer,
    palette: &Palette,
    (x, y): (u16, u16),
    accent_eighths: usize,
    accent_left: bool,
) {
    let continues_wide_glyph = x > 0
        && buffer
            .cell((x - 1, y))
            .is_some_and(|left| display_width(left.symbol()) > 1);
    let Some(cell) = buffer.cell_mut((x, y)) else {
        return;
    };
    let narrow = !continues_wide_glyph && display_width(cell.symbol()) == 1;
    match accent_eighths {
        0 => {}
        8 => {
            cell.set_style(filled_style(palette));
        }
        _ if !narrow => {
            if accent_eighths >= 4 {
                cell.set_style(filled_style(palette));
            }
        }
        _ => {
            let uncovered = cell.style().bg.unwrap_or(palette.panel_bg);
            let (left_eighths, fg, bg) = if accent_left {
                (accent_eighths, palette.accent, uncovered)
            } else {
                (8 - accent_eighths, uncovered, palette.accent)
            };
            cell.set_symbol(SWIPE_EDGE_BLOCKS[left_eighths]);
            cell.set_style(edge_style(fg, bg));
        }
    }
}

pub(crate) fn tab_bar_status_width(snapshot: &ClientShellSnapshot) -> u16 {
    let content = snapshot.tab_bar_right.iter().fold(0u16, |width, segment| {
        width.saturating_add(display_width(&segment.text))
    });
    let separators = snapshot.tab_bar_right.len().saturating_sub(1);
    content.saturating_add(
        display_width(&snapshot.tab_bar_right_separator)
            .saturating_mul(separators.min(u16::MAX as usize) as u16),
    )
}

fn tab_bar_status_area(snapshot: &ClientShellSnapshot, area: Rect) -> Option<Rect> {
    let width = tab_bar_status_width(snapshot);
    if width == 0 {
        return None;
    }
    let reserved = width.saturating_add(1);
    (area.width.saturating_sub(reserved) >= MIN_TAB_STRIP_WIDTH)
        .then(|| Rect::new(area.right().saturating_sub(width), area.y, width, 1))
}

fn tab_bar_content_area(snapshot: &ClientShellSnapshot, area: Rect) -> Rect {
    let reserved = tab_bar_status_area(snapshot, area)
        .map(|status| status.width.saturating_add(1))
        .unwrap_or(0);
    Rect {
        width: area.width.saturating_sub(reserved),
        ..area
    }
}

fn render_tab_bar_status(
    buffer: &mut Buffer,
    area: Rect,
    snapshot: &ClientShellSnapshot,
    palette: &Palette,
) {
    let Some(status) = tab_bar_status_area(snapshot, area) else {
        return;
    };
    let separator_width = display_width(&snapshot.tab_bar_right_separator);
    let mut x = status.x;
    for (index, segment) in snapshot.tab_bar_right.iter().enumerate() {
        if index > 0 && separator_width > 0 {
            put_text(
                buffer,
                x,
                area.y,
                separator_width,
                &snapshot.tab_bar_right_separator,
                Style::default().fg(palette.overlay0).bg(palette.panel_bg),
            );
            x = x.saturating_add(separator_width);
        }
        let width = display_width(&segment.text);
        let style = if segment.accent {
            Style::default()
                .fg(panel_contrast_fg(palette))
                .bg(palette.accent)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(palette.overlay1).bg(palette.panel_bg)
        };
        put_text(buffer, x, area.y, width, &segment.text, style);
        x = x.saturating_add(width);
    }
}

fn tab_drop_indicator_x(
    hits: &ShellHitMap,
    tabs: &[&ClientShellTab],
    insert_index: usize,
) -> Option<u16> {
    let visible = hits
        .tabs
        .iter()
        .filter_map(|(rect, tab_id)| {
            tabs.iter()
                .position(|tab| tab.tab_id == *tab_id)
                .map(|index| (index, *rect))
        })
        .collect::<Vec<_>>();
    let (first_index, first_rect) = *visible.first()?;
    let (last_index, last_rect) = *visible.last()?;
    if insert_index == 0 {
        return Some(if first_index == 0 {
            first_rect.x
        } else {
            hits.tab_scroll_left.right()
        });
    }
    if let Some((_, rect)) = visible.iter().find(|(index, _)| *index == insert_index) {
        return Some(rect.x.saturating_sub(1));
    }
    if insert_index >= tabs.len() {
        return Some(if last_index + 1 >= tabs.len() {
            last_rect.right()
        } else {
            hits.tab_scroll_right.x.saturating_sub(1)
        });
    }
    None
}

fn centered_tab_scroll(focused: usize, widths: &[u16], available: u16) -> usize {
    let mut best = focused;
    let mut best_distance = u16::MAX;
    for start in 0..=focused {
        let before = widths
            .iter()
            .copied()
            .enumerate()
            .skip(start)
            .take(focused.saturating_sub(start))
            .fold(0u16, |width, (_, tab)| width.saturating_add(tab + 1));
        if before >= available {
            continue;
        }
        let focused_width = widths[focused].min(available.saturating_sub(before));
        let center = before.saturating_mul(2).saturating_add(focused_width);
        let distance = center.abs_diff(available);
        if distance <= best_distance {
            best_distance = distance;
            best = start;
        }
    }
    best
}

fn max_tab_scroll(widths: &[u16], available: u16) -> usize {
    (0..widths.len())
        .find(|start| last_visible_tab(*start, widths, available) == widths.len().checked_sub(1))
        .unwrap_or(0)
}

fn last_visible_tab(start: usize, widths: &[u16], available: u16) -> Option<usize> {
    let mut remaining = available;
    let mut last = None;
    for (index, width) in widths.iter().copied().enumerate().skip(start) {
        if remaining == 0 {
            break;
        }
        last = Some(index);
        if width >= remaining {
            break;
        }
        remaining = remaining.saturating_sub(width.saturating_add(1));
    }
    last
}

fn tab_label(tab: &ClientShellTab) -> String {
    if tab.zoomed {
        format!("{} Z", tab.label)
    } else {
        tab.label.clone()
    }
}
