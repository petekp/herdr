use super::*;

/// A tab next to the focused one, rendered at the pane area's size so its
/// content can slide in behind a swipe. Fetched when the swipe picks it and
/// dropped when the swipe ends.
pub(super) struct NeighborSurface {
    pub(super) tab_id: String,
    pub(super) frame: FrameData,
}

/// Neighbor surfaces kept during one swipe.
const NEIGHBOR_SURFACES: usize = 4;

/// Columns of chrome between the outgoing and incoming content.
const TAB_SLIDE_GAP: u16 = 1;

/// The two surfaces a swipe moves across the pane area, and how far along.
pub(super) struct TabSlide<'a> {
    /// Content of the tab the swipe started on. `None` shows a blank surface.
    outgoing: Option<&'a FrameData>,
    /// Content of the tab the swipe is moving toward.
    incoming: Option<&'a FrameData>,
    /// Sign of the swipe: positive moves the content left, toward the next tab.
    direction: i32,
    progress: f32,
}

impl ClientShellState {
    fn focused_tab_id(&self) -> Option<&str> {
        self.snapshot.as_deref()?.focused_tab_id.as_deref()
    }

    /// Whether the pane area shows a swipe in motion rather than one tab at rest.
    pub(super) fn tab_slide_active(&self) -> bool {
        let Some(swipe) = self.tab_swipe.as_ref() else {
            return false;
        };
        let Some(target) = swipe.target.as_ref() else {
            return false;
        };
        if swipe.progress <= 0.0 {
            return false;
        }
        // Once the landed tab's own surface is showing, the swipe is at rest.
        !(swipe.progress >= 1.0 && self.focused_tab_id() == Some(target.tab_id.as_str()))
    }

    /// The slide to draw over `area`, with `surface` as the focused tab's content.
    pub(super) fn tab_slide<'a>(
        &'a self,
        surface: &'a FrameData,
        area: Rect,
    ) -> Option<TabSlide<'a>> {
        if !self.tab_slide_active() {
            return None;
        }
        let swipe = self.tab_swipe.as_ref()?;
        let target = swipe.target.as_ref()?;
        let focused = self.focused_tab_id();
        let frame_for = |tab_id: &str| {
            if focused == Some(tab_id) {
                return Some(surface);
            }
            self.neighbor_surfaces
                .iter()
                .find(|neighbor| neighbor.tab_id == tab_id)
                .map(|neighbor| &neighbor.frame)
                .filter(|frame| frame.width == area.width && frame.height == area.height)
        };
        Some(TabSlide {
            outgoing: frame_for(&swipe.origin_tab_id),
            incoming: frame_for(&target.tab_id),
            direction: swipe.direction(),
            progress: swipe.progress,
        })
    }

    /// Asks the server for `tab_id`'s surface unless it is cached or on its way.
    pub(super) fn request_neighbor_surface(
        &mut self,
        tab_id: &str,
        outcome: &mut ClientShellInput,
    ) {
        let cached = self
            .neighbor_surfaces
            .iter()
            .any(|neighbor| neighbor.tab_id == tab_id);
        let requested = self.pending_requests.values().any(|pending| {
            matches!(&pending.kind, PendingEndpointKind::NeighborSurface { tab_id: pending_tab } if pending_tab == tab_id)
        });
        if cached || requested {
            return;
        }
        let method =
            crate::api::schema::Method::ClientShellSurfaceRead(crate::api::schema::TabTarget {
                tab_id: tab_id.to_owned(),
            });
        // A server without the method gets a blank surface sliding in instead.
        if !self.endpoint_is_online(&self.active_endpoint_id)
            || !self.supports_endpoint_method(&method)
        {
            return;
        }
        self.push_endpoint_method_with_kind(
            method,
            PendingEndpointKind::NeighborSurface {
                tab_id: tab_id.to_owned(),
            },
            outcome,
        );
    }

    /// Keeps a surface the server rendered while the swipe is still in flight.
    /// Returns whether the pane area needs a repaint.
    pub(super) fn receive_neighbor_surface(
        &mut self,
        tab_id: &str,
        result: Result<crate::api::schema::ResponseResult, ClientShellEndpointError>,
    ) -> bool {
        let Ok(crate::api::schema::ResponseResult::ClientShellSurface {
            tab_id: rendered,
            cols,
            rows,
            lines,
        }) = result
        else {
            return false;
        };
        if rendered != tab_id || self.tab_swipe.is_none() {
            return false;
        }
        self.neighbor_surfaces
            .retain(|neighbor| neighbor.tab_id != tab_id);
        if self.neighbor_surfaces.len() >= NEIGHBOR_SURFACES {
            self.neighbor_surfaces.remove(0);
        }
        self.neighbor_surfaces.push(NeighborSurface {
            tab_id: tab_id.to_owned(),
            frame: decode_surface(cols, rows, &lines),
        });
        self.tab_slide_active()
    }
}

/// Rebuilds the cell grid from the runs `client_shell.surface.read` returns,
/// padding or trimming each row to `cols`.
fn decode_surface(
    cols: u16,
    rows: u16,
    lines: &[Vec<crate::api::schema::ClientShellSurfaceRun>],
) -> FrameData {
    let blank = blank_cell();
    let width = usize::from(cols);
    let mut cells = Vec::with_capacity(width * usize::from(rows));
    for row in 0..usize::from(rows) {
        let start = cells.len();
        for run in lines.get(row).map(Vec::as_slice).unwrap_or_default() {
            for symbol in &run.cells {
                if cells.len() - start >= width {
                    break;
                }
                cells.push(crate::protocol::CellData {
                    symbol: symbol.clone(),
                    fg: run.fg,
                    bg: run.bg,
                    modifier: run.modifier,
                    skip: false,
                    hyperlink: None,
                });
            }
        }
        cells.resize(start + width, blank.clone());
    }
    FrameData {
        cells,
        width: cols,
        height: rows,
        cursor: None,
        hyperlinks: Vec::new(),
        graphics: Vec::new(),
    }
}

fn blank_cell() -> crate::protocol::CellData {
    crate::protocol::CellData::from_ratatui_cell(&ratatui::buffer::Cell::new(" "))
}

/// Draws the swipe across `area`: the outgoing content moves out, a gap of
/// chrome follows it, and the incoming content moves in after the gap.
/// Content moves in whole columns; at `progress` 1 the incoming content is in place.
pub(super) fn compose_tab_slide(
    frame: &mut FrameData,
    area: Rect,
    slide: TabSlide<'_>,
    palette: &Palette,
) {
    let width = i32::from(area.width);
    let travel = width + i32::from(TAB_SLIDE_GAP);
    let direction = if slide.direction < 0 { -1 } else { 1 };
    // Columns the content has moved so far.
    let offset = ((slide.progress.clamp(0.0, 1.0) * travel as f32).round() as i32).clamp(0, travel);
    let outgoing_shift = -direction * offset;
    let incoming_shift = direction * (travel - offset);
    fill_blank(frame, area);
    if let Some(outgoing) = slide.outgoing {
        blit_shifted(frame, outgoing, area, outgoing_shift);
    }
    if let Some(incoming) = slide.incoming {
        blit_shifted(frame, incoming, area, incoming_shift);
    }
    // The seam trails whichever surface leads in the swipe direction.
    let leading_shift = if direction > 0 {
        outgoing_shift
    } else {
        incoming_shift
    };
    paint_gap(frame, area, leading_shift + width, palette);
    frame.cursor = None;
    frame.graphics.clear();
}

fn fill_blank(frame: &mut FrameData, area: Rect) {
    let blank = blank_cell();
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            if let Some(cell) = frame_cell_mut(frame, x, y) {
                *cell = blank.clone();
            }
        }
    }
}

fn paint_gap(frame: &mut FrameData, area: Rect, start: i32, palette: &Palette) {
    let mut cell = ratatui::buffer::Cell::new("│");
    cell.set_style(
        Style::default()
            .fg(palette.surface_dim)
            .bg(palette.panel_bg),
    );
    let gap = crate::protocol::CellData::from_ratatui_cell(&cell);
    for row in 0..area.height {
        for column in start..start + i32::from(TAB_SLIDE_GAP) {
            let Ok(column) = u16::try_from(column) else {
                continue;
            };
            if column >= area.width {
                continue;
            }
            if let Some(cell) = frame_cell_mut(frame, area.x + column, area.y + row) {
                *cell = gap.clone();
            }
        }
    }
}

fn frame_cell_mut(frame: &mut FrameData, x: u16, y: u16) -> Option<&mut crate::protocol::CellData> {
    if x >= frame.width || y >= frame.height {
        return None;
    }
    frame
        .cells
        .get_mut(usize::from(y) * usize::from(frame.width) + usize::from(x))
}

/// Copies `source` into `area` moved `shift` columns sideways, dropping cells
/// that fall outside. A wide glyph cut by an edge becomes a blank so it cannot
/// spill past the edge.
pub(super) fn blit_shifted(target: &mut FrameData, source: &FrameData, area: Rect, shift: i32) {
    let copy_height = source.height.min(area.height);
    let hyperlink_base = target.hyperlinks.len() as u32;
    target.hyperlinks.extend(source.hyperlinks.iter().cloned());
    let source_width = usize::from(source.width);
    let wide = |cell: &crate::protocol::CellData| super::render::display_width(&cell.symbol) > 1;

    for row in 0..copy_height {
        let cells =
            &source.cells[usize::from(row) * source_width..usize::from(row + 1) * source_width];
        for (col, source_cell) in cells.iter().enumerate() {
            let x = col as i32 + shift;
            if x < 0 || x >= i32::from(area.width) {
                continue;
            }
            let Some(target_cell) = frame_cell_mut(target, area.x + x as u16, area.y + row) else {
                continue;
            };
            *target_cell = source_cell.clone();
            target_cell.hyperlink = source_cell.hyperlink.and_then(|index| {
                ((index as usize) < source.hyperlinks.len()).then_some(hyperlink_base + index)
            });
            let cut_leading = wide(source_cell) && x + 1 >= i32::from(area.width);
            let cut_trailing = x == 0 && col > 0 && wide(&cells[col - 1]);
            if cut_leading || cut_trailing {
                target_cell.symbol = " ".into();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(lines: &[&str]) -> FrameData {
        FrameData::from_ratatui_buffer_with_hyperlinks(
            &Buffer::with_lines(lines.iter().copied()),
            None,
            &[],
        )
    }

    fn symbols(frame: &FrameData) -> Vec<&str> {
        frame
            .cells
            .iter()
            .map(|cell| cell.symbol.as_str())
            .collect()
    }

    #[test]
    fn a_wide_glyph_cut_by_an_edge_becomes_a_blank() {
        let source = frame(&["a本b"]);
        let area = Rect::new(0, 0, 4, 1);

        let mut right = frame(&["    "]);
        blit_shifted(&mut right, &source, area, 2);
        assert_eq!(symbols(&right), [" ", " ", "a", " "]);

        let mut left = frame(&["    "]);
        blit_shifted(&mut left, &source, area, -2);
        assert_eq!(symbols(&left), [" ", "b", " ", " "]);

        let mut whole = frame(&["    "]);
        blit_shifted(&mut whole, &source, area, -1);
        assert_eq!(symbols(&whole), ["本", " ", "b", " "]);
    }

    #[test]
    fn decoded_rows_are_padded_and_trimmed_to_the_surface_size() {
        let run = |cells: &[&str]| crate::api::schema::ClientShellSurfaceRun {
            cells: cells.iter().map(|cell| (*cell).to_owned()).collect(),
            fg: 1,
            bg: 2,
            modifier: 3,
        };
        let decoded = decode_surface(3, 2, &[vec![run(&["a"]), run(&["b", "c", "d"])]]);
        assert_eq!(symbols(&decoded), ["a", "b", "c", " ", " ", " "]);
        assert_eq!(decoded.cells[0].fg, 1);
        assert_eq!(decoded.cells[3], blank_cell());
    }

    #[test]
    fn the_outgoing_content_leaves_ahead_of_a_seam_with_the_neighbor_behind_it() {
        let area = Rect::new(0, 0, 6, 1);
        let outgoing = frame(&["oooooo"]);
        let incoming = frame(&["nnnnnn"]);
        let palette = Palette::catppuccin();
        let compose = |direction: i32, progress: f32| {
            let mut composed = frame(&["      "]);
            compose_tab_slide(
                &mut composed,
                area,
                TabSlide {
                    outgoing: Some(&outgoing),
                    incoming: Some(&incoming),
                    direction,
                    progress,
                },
                &palette,
            );
            symbols(&composed).concat()
        };

        assert_eq!(compose(1, 0.0), "oooooo");
        assert_eq!(
            compose(1, 0.5),
            "oo│nnn",
            "the seam follows the outgoing content out"
        );
        assert_eq!(compose(1, 1.0), "nnnnnn");
        assert_eq!(
            compose(-1, 0.5),
            "nnn│oo",
            "a swipe the other way enters from the left"
        );
    }

    #[test]
    fn surface_runs_round_trip_through_the_server_encoder() {
        let mut buffer = Buffer::with_lines(["ab本", "cd  "]);
        buffer[(1, 0)].set_style(Style::default().fg(ratatui::style::Color::Red));
        buffer[(0, 1)].set_style(Style::default().add_modifier(Modifier::BOLD));
        let original = FrameData::from_ratatui_buffer_with_hyperlinks(&buffer, None, &[]);

        let lines = crate::server::client_shell::surface_lines(&original);
        assert_eq!(lines[0].len(), 3, "one run per style change");
        let decoded = decode_surface(original.width, original.height, &lines);

        assert_eq!(decoded.cells, original.cells);
    }
}
