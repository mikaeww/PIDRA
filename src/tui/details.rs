use ratatui::{
    Frame,
    layout::{Constraint, Layout},
    text::{Line, Span},
    widgets::{Paragraph, Wrap},
};

use crate::{
    app::App,
    control::risk::{RiskAssessment, RiskRating, assess_termination},
    process::{ApplicationResources, ProcessSnapshot, format::masked_command},
    tui::{RenderOptions, process_table::format_bytes, theme::Palette},
};

pub fn render(frame: &mut Frame<'_>, app: &App, options: RenderOptions) {
    let palette = Palette::new(options.no_color);
    let rows = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(0),
        Constraint::Length(1),
        Constraint::Length(2),
        Constraint::Length(1),
    ])
    .split(frame.area());
    let Some(process) = app.selected_detail_process() else {
        frame.render_widget(
            Paragraph::new("Process unavailable\n\nEsc Back").style(palette.header()),
            frame.area(),
        );
        return;
    };

    let application_root = app.details_root.unwrap_or(process.identity);
    let root = app.process_by_identity(application_root).unwrap_or(process);
    let resources = app.application_resources(application_root);
    let risk = assess_termination(
        process,
        &app.all_processes,
        i32::try_from(std::process::id()).unwrap_or(i32::MAX),
    );
    frame.render_widget(
        Paragraph::new(vec![
            Line::styled(
                format!("{} / Details", app.display_name(root)),
                palette.header(),
            ),
            Line::styled(
                if app.details_technical {
                    "Tab Overview   TECHNICAL"
                } else {
                    "OVERVIEW   Tab Technical"
                },
                palette.muted(),
            ),
        ]),
        rows[0],
    );

    let lines = if app.details_technical {
        technical_lines(app, process, resources, &risk, &palette)
    } else {
        overview_lines(app, process, resources, &risk, options, &palette)
    };
    super::render_scrollable(frame, rows[1], lines, &app.info_scroll);

    let freeze = if process.state == crate::process::ProcessState::Stopped {
        "Resume"
    } else {
        "Freeze"
    };
    frame.render_widget(
        Paragraph::new(format!(
            "R Restart   F {freeze}   T Stop   Shift+K Force stop"
        ))
        .style(palette.accent()),
        rows[2],
    );
    let navigation = if frame.area().width < 58 {
        "Esc Back   Tab View   PgUp/PgDn Scroll\nUp/Down Select   Left/Right Expand"
    } else if options.ascii {
        "Esc Back   Tab View   PgUp/PgDn Scroll   Up/Down Select   Left/Right Expand"
    } else {
        "Esc Back   Tab View   PgUp/PgDn Scroll   ↑↓ Select   ←→ Expand"
    };
    frame.render_widget(
        Paragraph::new(navigation)
            .style(palette.footer())
            .wrap(Wrap { trim: false }),
        rows[3],
    );
    frame.render_widget(
        Paragraph::new(app.status.as_str()).style(palette.status()),
        rows[4],
    );
}

fn overview_lines<'a>(
    app: &'a App,
    process: &'a ProcessSnapshot,
    resources: ApplicationResources,
    risk: &RiskAssessment,
    options: RenderOptions,
    palette: &Palette,
) -> Vec<Line<'a>> {
    let mut lines = vec![
        Line::styled("APPLICATION", palette.table_header()),
        Line::from(format!(
            "Memory {}   CPU {:.1}%   {} processes",
            format_bytes(resources.preferred_memory_bytes()),
            resources.cpu_percent,
            resources.process_count
        )),
        Line::styled(
            format_trend(app.resource_trend(app.details_root.unwrap_or(process.identity))),
            palette.muted(),
        ),
        Line::from(""),
        Line::styled("PROCESS TREE", palette.table_header()),
    ];
    let nodes = app.detail_nodes();
    let capacity = 8usize;
    let start = app
        .details_selected
        .saturating_sub(capacity.saturating_sub(1));
    for (index, node) in nodes.iter().skip(start).take(capacity).enumerate() {
        let absolute = start + index;
        let marker = match (node.has_children, node.expanded, options.ascii) {
            (true, true, true) => "-",
            (true, false, true) => "+",
            (false, _, true) => "*",
            (true, true, false) => "▾",
            (true, false, false) => "▸",
            (false, _, false) => "•",
        };
        let selector = if absolute == app.details_selected {
            if options.ascii { ">" } else { "›" }
        } else {
            " "
        };
        let label = app.process_by_identity(node.identity).map_or_else(
            || format!("PID {} unavailable", node.identity.pid),
            |item| format!("{}   PID {}", item.name, item.identity.pid),
        );
        lines.push(Line::from(vec![
            Span::styled(
                format!("{selector}{}", "  ".repeat(node.depth)),
                if absolute == app.details_selected {
                    palette.accent()
                } else {
                    palette.base()
                },
            ),
            Span::raw(format!("{marker} {label}")),
        ]));
    }
    lines.extend([
        Line::from(""),
        Line::styled("SELECTED", palette.table_header()),
        Line::from(format!(
            "{}   PID {}   {}",
            process.name,
            process.identity.pid,
            process.state.label()
        )),
        Line::from(format!(
            "Memory {}   CPU {:.1}%",
            format_bytes(process.pss_bytes.unwrap_or(process.rss_bytes)),
            process.cpu_percent
        )),
        Line::styled("Actions affect this process only.", palette.muted()),
        Line::from(""),
        Line::styled("SAFETY", palette.table_header()),
        Line::styled(safety_summary(risk.rating), palette.warning()),
    ]);
    if app
        .details_root
        .and_then(|identity| app.gui_classifications.get(&identity))
        .is_some_and(|classification| {
            classification.confidence != crate::process::GuiConfidence::Confirmed
        })
    {
        lines.push(Line::styled(
            "No window confirmed. Tab shows detection evidence.",
            palette.muted(),
        ));
    }
    lines
}

fn technical_lines<'a>(
    app: &'a App,
    process: &'a ProcessSnapshot,
    resources: ApplicationResources,
    risk: &RiskAssessment,
    palette: &Palette,
) -> Vec<Line<'a>> {
    let parent = process.parent_pid.and_then(|pid| {
        app.all_processes
            .iter()
            .find(|candidate| candidate.identity.pid == pid)
    });
    let pss = process
        .pss_bytes
        .map_or_else(|| "unavailable".to_owned(), format_bytes);
    let mut lines = vec![
        Line::styled("PROCESS", palette.table_header()),
        Line::from(format!(
            "PID {}   UID {}   Start {}",
            process.identity.pid, process.uid, process.identity.start_time_ticks
        )),
        Line::from(format!(
            "Parent: {}",
            parent.map_or_else(
                || process
                    .parent_pid
                    .map_or("none".to_owned(), |pid| pid.to_string()),
                |parent| format!("{} ({})", parent.name, parent.identity.pid)
            )
        )),
        Line::from(format!(
            "Executable: {}",
            process.executable.as_deref().map_or_else(
                || "unavailable".to_owned(),
                |path| path.display().to_string()
            )
        )),
        Line::from(format!(
            "Command: {}",
            if process.command.is_empty() {
                "unavailable".to_owned()
            } else {
                masked_command(&process.command)
            }
        )),
        Line::from(format!(
            "Working directory: {}",
            process.cwd.as_deref().map_or_else(
                || "unavailable".to_owned(),
                |path| path.display().to_string()
            )
        )),
        Line::from(""),
        Line::styled("RESOURCES", palette.table_header()),
        Line::from(format!(
            "CPU {:.1}%   RSS {}   PSS {}   Threads {}",
            process.cpu_percent,
            format_bytes(process.rss_bytes),
            pss,
            process.thread_count
        )),
        Line::from(format!(
            "Read {}   Write {}",
            format_rate(process.read_rate_bytes),
            format_rate(process.write_rate_bytes)
        )),
        Line::from(format!(
            "Application: {} processes   Memory {}   CPU {:.1}%",
            resources.process_count,
            format_bytes(resources.preferred_memory_bytes()),
            resources.cpu_percent
        )),
        Line::from(""),
        Line::styled("DETECTION", palette.table_header()),
    ];
    if let Some(classification) = app.developer_classifications.get(&process.identity) {
        lines.push(Line::from(format!(
            "{}   {}",
            classification.kind.label(),
            classification.endpoints.join(", ")
        )));
        lines.push(Line::from(classification.evidence.join("; ")));
    } else if let Some(classification) = app
        .details_root
        .and_then(|identity| app.gui_classifications.get(&identity))
    {
        lines.push(Line::from(format!(
            "{:?}   {}",
            classification.confidence,
            classification
                .application_scope
                .as_deref()
                .unwrap_or("no application scope")
        )));
        lines.push(Line::from(classification.evidence.join("; ")));
    } else {
        lines.push(Line::from("No GUI classification for this process."));
    }
    lines.extend([
        Line::from(format!(
            "Restart: {}",
            app.restart_source_for(process.identity).summary()
        )),
        Line::from(format!("Cgroup: {}", process.cgroups.join("; "))),
        Line::from(""),
        Line::styled("SAFETY", palette.table_header()),
        Line::styled(
            format!(
                "{}   Confidence {}",
                risk.rating.label(),
                risk.confidence.label()
            ),
            palette.header(),
        ),
        Line::from(risk.evidence.join("; ")),
        Line::from(risk.warning.clone()),
        Line::from(format!(
            "Last action: {}",
            app.latest_action_for(process.identity)
                .unwrap_or("none in this session")
        )),
    ]);
    lines
}

fn safety_summary(rating: RiskRating) -> &'static str {
    match rating {
        RiskRating::Protected => "Protected process. Actions are blocked.",
        RiskRating::Unknown => "State uncertain. Check Technical before acting.",
        RiskRating::Caution | RiskRating::LikelySafe | RiskRating::CloseFromApplicationFirst => {
            "Save your work and close the app normally first."
        }
    }
}

fn format_rate(rate: Option<f64>) -> String {
    rate.map_or_else(
        || "unavailable".to_owned(),
        |rate| format!("{}/s", format_bytes(rate.max(0.0) as u64)),
    )
}

fn format_trend(trend: Option<crate::process::ResourceTrend>) -> String {
    trend.map_or_else(
        || "30s trend collecting…".to_owned(),
        |trend| {
            let direction = match trend.memory_delta_bytes.cmp(&0) {
                std::cmp::Ordering::Greater => "+",
                std::cmp::Ordering::Less => "-",
                std::cmp::Ordering::Equal => "",
            };
            let magnitude =
                u64::try_from(trend.memory_delta_bytes.unsigned_abs()).unwrap_or(u64::MAX);
            format!(
                "{:.0}s: memory {direction}{}   CPU avg {:.1}%",
                trend.duration.as_secs_f64(),
                format_bytes(magnitude),
                trend.average_cpu_percent
            )
        },
    )
}
