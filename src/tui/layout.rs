use ratatui::layout::{Constraint, Direction, Layout, Rect};

#[derive(Debug, Clone, Copy)]
pub struct Areas {
    pub header: Rect,
    pub table: Rect,
    pub actions: Rect,
    pub status: Rect,
    pub footer: Rect,
}

pub fn areas(area: Rect, process_count: usize) -> Areas {
    let table_height = u16::try_from(process_count)
        .unwrap_or(u16::MAX)
        .saturating_add(2)
        .min(area.height.saturating_sub(7));
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(table_height),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(area);

    Areas {
        header: rows[0],
        table: rows[2],
        actions: rows[4],
        status: rows[6],
        footer: rows[7],
    }
}

pub fn action_areas(area: Rect, name: &str) -> std::rc::Rc<[Rect]> {
    let compact = area.width < 50;
    let action_widths = if compact { [5, 5, 5] } else { [13, 10, 13] };
    let action_width = action_widths.iter().sum::<u16>();
    let name_width = u16::try_from(name.chars().count())
        .unwrap_or(u16::MAX)
        .saturating_add(3)
        .min(area.width.saturating_sub(action_width));
    Layout::horizontal([
        Constraint::Length(name_width),
        Constraint::Length(action_widths[0]),
        Constraint::Length(action_widths[1]),
        Constraint::Length(action_widths[2]),
        Constraint::Min(0),
    ])
    .split(area)
}
