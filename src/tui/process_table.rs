use ratatui::{
    Frame,
    layout::{Constraint, Rect},
    text::{Line, Span},
    widgets::{Cell, Paragraph, Row, Table},
};

use crate::{
    app::{App, FocusColumn, SortMode},
    process::ProcessSnapshot,
    tui::{RenderOptions, TableHit, layout, theme::Palette},
};

pub fn render(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &App,
    options: RenderOptions,
    palette: &Palette,
) {
    let show_bars = area.width >= 64;
    let mut widths = vec![
        Constraint::Min(12),
        Constraint::Length(8),
        Constraint::Length(12),
    ];
    let mut headers = vec![
        Cell::from(sort_header(
            "PROCESS NAME",
            app.sort_mode == SortMode::Name,
            options,
        )),
        Cell::from(
            Line::from(sort_header("PID", app.sort_mode == SortMode::Pid, options)).right_aligned(),
        ),
        Cell::from(
            Line::from(sort_header(
                "MEM P/R",
                app.sort_mode == SortMode::Memory,
                options,
            ))
            .right_aligned(),
        ),
    ];
    if show_bars {
        widths.push(Constraint::Length(12));
        headers.push(Cell::from("REL. MEM"));
    }
    let maximum = app
        .processes
        .iter()
        .map(|process| {
            app.application_resources(process.identity)
                .preferred_memory_bytes()
        })
        .max()
        .unwrap_or(0)
        .max(1);
    let (start, end) = visible_range(app.processes.len(), app.selected, area.height);
    let rows = app.processes[start..end]
        .iter()
        .enumerate()
        .map(|(offset, process)| {
            let selected = start + offset == app.selected;
            let marker = if selected {
                if options.ascii { ">" } else { "›" }
            } else {
                " "
            };
            let name = Line::from(Span::styled(
                format!("{marker}{}", app.display_name(process)),
                if selected {
                    palette.selected_row()
                } else {
                    palette.base()
                },
            ));
            let mut cells = vec![
                Cell::from(name),
                Cell::from(Line::from(process.identity.pid.to_string()).right_aligned())
                    .style(palette.muted()),
                Cell::from(Line::from(format_application_memory(app, process)).right_aligned()),
            ];
            if show_bars {
                let memory = app
                    .application_resources(process.identity)
                    .preferred_memory_bytes();
                cells.push(Cell::from(meter(
                    memory as f64 / maximum as f64,
                    10,
                    options.ascii,
                    palette,
                )));
            }
            Row::new(cells).style(palette.base())
        });
    frame.render_widget(
        Table::new(rows, widths)
            .header(
                Row::new(headers)
                    .style(palette.table_header())
                    .bottom_margin(1),
            )
            .column_spacing(2),
        area,
    );
    if app.processes.is_empty() {
        let message = if app.search_query.is_empty() {
            "No matching applications"
        } else {
            "No matches - / edit search"
        };
        frame.render_widget(
            Paragraph::new(message).style(palette.muted()),
            Rect::new(
                area.x,
                area.y.saturating_add(1),
                area.width,
                area.height.saturating_sub(1),
            ),
        );
    }
}

pub fn render_actions(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &App,
    options: RenderOptions,
    palette: &Palette,
) {
    let Some(process) = app.processes.get(app.selected) else {
        return;
    };
    let name = app.display_name(process);
    let columns = layout::action_areas(area, name);
    frame.render_widget(
        Paragraph::new(format!("{name} {}", if options.ascii { "->" } else { "→" }))
            .style(palette.header()),
        columns[0],
    );
    for (index, focus) in [
        FocusColumn::Restart,
        FocusColumn::Stop,
        FocusColumn::Details,
    ]
    .into_iter()
    .enumerate()
    {
        let available = focus != FocusColumn::Restart
            || app.restart_source_for(process.identity).is_available();
        let label = match (focus, area.width < 50, available) {
            (FocusColumn::Restart, true, true) => "[R]",
            (FocusColumn::Restart, true, false) => "R --",
            (FocusColumn::Restart, false, true) => "[R] Restart",
            (FocusColumn::Restart, false, false) => "[R] --",
            (FocusColumn::Stop, true, _) => "[S]",
            (FocusColumn::Stop, false, _) => "[S] Stop",
            (FocusColumn::Details, true, _) => "[D]",
            (FocusColumn::Details, false, _) => "[D] Details",
        };
        let style = if focus == app.focus {
            palette.focused_action()
        } else if available {
            palette.base()
        } else {
            palette.muted()
        };
        frame.render_widget(
            Paragraph::new(label).style(style).centered(),
            columns[index + 1],
        );
    }
}

pub(super) fn meter(fraction: f64, width: usize, ascii: bool, palette: &Palette) -> Line<'static> {
    let filled = (fraction.clamp(0.0, 1.0) * width as f64).round() as usize;
    Line::from(vec![
        Span::styled(
            (if ascii { "|" } else { "━" }).repeat(filled),
            palette.accent(),
        ),
        Span::styled(
            (if ascii { "." } else { "·" }).repeat(width - filled),
            palette.muted(),
        ),
    ])
}

fn visible_range(total: usize, selected: usize, area_height: u16) -> (usize, usize) {
    let capacity = usize::from(area_height.saturating_sub(2));
    let selected = selected.min(total.saturating_sub(1));
    let start = selected.saturating_sub(capacity.saturating_sub(1));
    let end = (start + capacity).min(total);
    (start, end)
}

pub fn hit_test(area: Rect, app: &App, x: u16, y: u16) -> Option<TableHit> {
    let first_data_row = area.y.saturating_add(2);
    if y < first_data_row || y >= area.bottom() || x < area.x || x >= area.right() {
        return None;
    }
    let (start, end) = visible_range(app.processes.len(), app.selected, area.height);
    let row = start + usize::from(y - first_data_row);
    (row < end).then_some(TableHit { row, focus: None })
}

fn sort_header(label: &str, active: bool, options: RenderOptions) -> String {
    if active {
        format!("{label}{}", if options.ascii { "v" } else { "↓" })
    } else {
        label.to_owned()
    }
}

fn format_application_memory(app: &App, process: &ProcessSnapshot) -> String {
    let resources = app.application_resources(process.identity);
    format!(
        "{} {}",
        format_bytes(resources.preferred_memory_bytes()),
        if resources.has_complete_pss() {
            "P"
        } else {
            "R"
        }
    )
}

pub(crate) fn format_bytes(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = KIB * 1024.0;
    const GIB: f64 = MIB * 1024.0;
    const TIB: f64 = GIB * 1024.0;
    let bytes_float = bytes as f64;

    if bytes_float >= TIB {
        format!("{:.1} TB", bytes_float / TIB)
    } else if bytes_float >= GIB {
        format!("{:.1} GB", bytes_float / GIB)
    } else if bytes_float >= MIB {
        format!("{:.0} MB", bytes_float / MIB)
    } else if bytes_float >= KIB {
        format!("{:.0} KB", bytes_float / KIB)
    } else {
        format!("{bytes} B")
    }
}

#[cfg(test)]
mod tests {
    use ratatui::layout::Rect;

    use crate::{app::App, tui::TableHit};

    use super::{format_bytes, hit_test, visible_range};

    #[test]
    fn formats_binary_byte_units() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(1_024), "1 KB");
        assert_eq!(format_bytes(1_048_576), "1 MB");
        assert_eq!(format_bytes(1_073_741_824), "1.0 GB");
    }

    #[test]
    fn keeps_selected_row_inside_large_viewports() {
        assert_eq!(visible_range(10_000, 9_999, 20), (9_982, 10_000));
        assert_eq!(visible_range(10_000, 5, 20), (0, 18));
    }

    #[test]
    fn maps_pointer_coordinates_to_the_shared_action_model() {
        let app = App::fixture();
        let area = Rect::new(0, 2, 80, 18);

        assert_eq!(
            hit_test(area, &app, 74, 4),
            Some(TableHit {
                row: 0,
                focus: None,
            })
        );
        assert_eq!(
            hit_test(area, &app, 3, 5),
            Some(TableHit {
                row: 1,
                focus: None,
            })
        );
    }
}
