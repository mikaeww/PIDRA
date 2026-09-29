mod confirm;
mod details;
mod help;
mod history;
mod layout;
mod process_table;
mod restart_confirm;
mod theme;

use ratatui::{
    Frame,
    layout::{Constraint, Layout},
    text::{Line, Span},
    widgets::{Block, Paragraph, Wrap},
};

use crate::app::{App, AppView, FocusColumn};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TableHit {
    pub row: usize,
    pub focus: Option<FocusColumn>,
}

#[derive(Debug, Clone, Copy)]
pub struct RenderOptions {
    pub ascii: bool,
    pub no_color: bool,
}

pub fn render(frame: &mut Frame<'_>, app: &App, options: RenderOptions) {
    let palette = theme::Palette::new(options.no_color);
    frame.render_widget(Block::default().style(palette.base()), frame.area());
    match app.view {
        AppView::Details => {
            details::render(frame, app, options);
            return;
        }
        AppView::Confirm => {
            confirm::render(frame, app, options);
            return;
        }
        AppView::RestartConfirm => {
            restart_confirm::render(frame, app, options);
            return;
        }
        AppView::History => {
            history::render(frame, app, options);
            return;
        }
        AppView::Help => {
            help::render(frame, options);
            return;
        }
        AppView::Table | AppView::Developer => {}
    }
    let areas = layout::areas(frame.area(), app.processes.len());
    let metrics_width = if frame.area().width >= 80 {
        32
    } else if frame.area().width >= 55 {
        20
    } else {
        0
    };
    let header_columns =
        Layout::horizontal([Constraint::Min(0), Constraint::Length(metrics_width)])
            .split(areas.header);
    let mut title = vec![Span::styled("PIDRA   ", palette.header())];
    title.push(Span::styled(
        format!("Apps {}", app.graphical_total()),
        if app.developer_layer_active() {
            palette.muted()
        } else {
            palette.header()
        },
    ));
    title.push(Span::styled(" / ", palette.muted()));
    title.push(Span::styled(
        format!("Dev {}", app.developer_total()),
        if app.developer_layer_active() {
            palette.header()
        } else {
            palette.muted()
        },
    ));
    if !app.search_query.is_empty() || app.searching {
        title.push(Span::styled(
            format!("  /{}", app.search_query),
            palette.header(),
        ));
    }
    frame.render_widget(Paragraph::new(Line::from(title)), header_columns[0]);
    let mut system = Vec::new();
    for (label, value) in [
        ("CPU", app.system_metrics.cpu_percent),
        ("RAM", app.system_metrics.memory_used_percent),
    ] {
        system.push(Span::styled(
            format!("{label} {} ", percent(value)),
            palette.muted(),
        ));
        if metrics_width == 32 {
            if let Some(value) = value {
                system.extend(
                    process_table::meter(f64::from(value) / 100.0, 5, options.ascii, &palette)
                        .spans,
                );
            } else {
                system.push(Span::raw("     "));
            }
            system.push(Span::raw("  "));
        }
    }
    frame.render_widget(
        Paragraph::new(Line::from(system)).right_aligned(),
        header_columns[1],
    );

    process_table::render(frame, areas.table, app, options, &palette);

    frame.render_widget(
        Paragraph::new(app.status.as_str()).style(palette.status()),
        areas.status,
    );
    process_table::render_actions(frame, areas.actions, app, options, &palette);
    let footer = if app.searching {
        if options.ascii {
            "SEARCH: TYPE NAME OR PID   BACKSPACE DELETE   ENTER/ESC CLOSE"
        } else {
            "SEARCH: TYPE NAME OR PID   ⌫ DELETE   ENTER/ESC CLOSE"
        }
    } else if areas.footer.width < 60 {
        "Enter use  / Search  ? Help  Q Quit"
    } else {
        "Enter use   / Search   O Sort   V Apps/Dev   ? Help   Q Quit"
    };
    frame.render_widget(Paragraph::new(footer).style(palette.footer()), areas.footer);
}

fn render_scrollable(
    frame: &mut Frame<'_>,
    area: ratatui::layout::Rect,
    lines: Vec<Line<'_>>,
    scroll: &std::cell::Cell<u16>,
) {
    let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });
    let maximum = paragraph
        .line_count(area.width)
        .saturating_sub(usize::from(area.height));
    let offset = scroll.get().min(u16::try_from(maximum).unwrap_or(u16::MAX));
    scroll.set(offset);
    frame.render_widget(paragraph.scroll((offset, 0)), area);
}

fn percent(value: Option<f32>) -> String {
    value.map_or_else(|| "--".to_owned(), |value| format!("{value:.0}%"))
}

#[must_use]
pub fn table_hit(area: ratatui::layout::Rect, app: &App, x: u16, y: u16) -> Option<TableHit> {
    if !matches!(app.view, AppView::Table | AppView::Developer) {
        return None;
    }
    let areas = layout::areas(area, app.processes.len());
    if let Some(hit) = process_table::hit_test(areas.table, app, x, y) {
        return Some(hit);
    }
    let process = app.processes.get(app.selected)?;
    let columns = layout::action_areas(areas.actions, app.display_name(process));
    [
        FocusColumn::Restart,
        FocusColumn::Stop,
        FocusColumn::Details,
    ]
    .into_iter()
    .enumerate()
    .find_map(|(index, focus)| {
        columns[index + 1]
            .contains((x, y).into())
            .then_some(TableHit {
                row: app.selected,
                focus: Some(focus),
            })
    })
}

#[cfg(test)]
mod tests {
    use std::{ffi::OsString, path::PathBuf};

    use ratatui::{Terminal, backend::TestBackend};

    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    use crate::{
        app::{App, AppView, FocusColumn},
        process::{
            DeveloperClassification, DeveloperKind, ProcessSnapshot, ScanBatch, cpu::SystemMetrics,
        },
    };

    use super::{RenderOptions, render};

    #[test]
    fn shared_actions_follow_selection_and_match_pointer_hits_at_each_width() {
        use super::{TableHit, layout, table_hit};
        use ratatui::{
            layout::Rect,
            style::{Color, Modifier},
        };

        for width in [20, 44, 64, 100] {
            for no_color in [false, true] {
                let area = Rect::new(0, 0, width, 18);
                let mut app = App::fixture();
                app.selected = 1;
                app.focus = FocusColumn::Restart;
                let mut terminal = Terminal::new(TestBackend::new(width, 18)).unwrap();
                let options = RenderOptions {
                    ascii: true,
                    no_color,
                };
                terminal.draw(|frame| render(frame, &app, options)).unwrap();
                let areas = layout::areas(area, app.processes.len());
                let actions = layout::action_areas(
                    areas.actions,
                    app.display_name(&app.processes[app.selected]),
                );
                assert_eq!(actions[1].x, actions[0].right());
                if width >= 64 {
                    assert!(actions[3].right() < area.right());
                }
                for (index, focus) in [
                    FocusColumn::Restart,
                    FocusColumn::Stop,
                    FocusColumn::Details,
                ]
                .into_iter()
                .enumerate()
                {
                    let cell = actions[index + 1];
                    assert_eq!(
                        table_hit(area, &app, cell.x, cell.y),
                        Some(TableHit {
                            row: 1,
                            focus: Some(focus)
                        })
                    );
                }
                assert_eq!(
                    table_hit(area, &app, area.right() - 1, areas.table.y + 2),
                    Some(TableHit {
                        row: 0,
                        focus: None
                    })
                );
                assert_eq!(table_hit(area, &app, 0, areas.table.bottom()), None);
                if width >= 64 {
                    let text = terminal.backend().to_string();
                    assert_eq!(text.matches("[S] Stop").count(), 1);
                    assert_eq!(text.matches("[D] Details").count(), 1);
                    assert!(text.contains("REL. MEM"));
                }
                let buffer = terminal.backend().buffer();
                for cell in buffer.content() {
                    assert_eq!(cell.fg, Color::Reset);
                    assert_eq!(cell.bg, Color::Reset);
                }
                let selected_y = areas.table.y + 3;
                assert!(
                    buffer[(0, selected_y)]
                        .modifier
                        .contains(Modifier::REVERSED)
                );
                if width >= 44 {
                    assert!(
                        !buffer[(20, selected_y)]
                            .modifier
                            .contains(Modifier::REVERSED)
                    );
                }
                assert!(
                    !buffer[(width - 1, selected_y)]
                        .modifier
                        .contains(Modifier::REVERSED)
                );
                let hit = table_hit(area, &app, actions[3].x, actions[3].y).unwrap();
                app.select_from_pointer(hit.row, hit.focus);
                assert_eq!(app.view, AppView::Table);
                app.select_from_pointer(hit.row, hit.focus);
                assert_eq!(app.view, AppView::Details);
                app = App::new();
                terminal.draw(|frame| render(frame, &app, options)).unwrap();
                assert_eq!(table_hit(area, &app, actions[2].x, actions[2].y), None);
            }
        }
    }

    #[test]
    fn renders_the_phase_zero_surface() {
        let backend = TestBackend::new(100, 14);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        let app = App::fixture();

        terminal
            .draw(|frame| {
                render(
                    frame,
                    &app,
                    RenderOptions {
                        ascii: false,
                        no_color: true,
                    },
                );
            })
            .expect("render fixture");

        let rendered = terminal.backend().to_string();
        assert!(rendered.contains("PIDRA"));
        assert!(rendered.contains("PROCESS NAME"));
        assert!(rendered.contains("firefox"));
        assert!(rendered.contains("Enter use"));
    }

    #[test]
    fn renders_a_narrow_ascii_no_color_surface() {
        let backend = TestBackend::new(44, 10);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        let app = App::fixture();

        terminal
            .draw(|frame| {
                render(
                    frame,
                    &app,
                    RenderOptions {
                        ascii: true,
                        no_color: true,
                    },
                );
            })
            .expect("render compact fixture");

        let rendered = terminal.backend().to_string();
        assert!(rendered.contains("PIDRA"));
        assert!(rendered.contains("PROCESS NAME"));
        assert!(rendered.contains("nira"));
    }

    #[test]
    fn developer_layer_and_details_show_the_classification_evidence() {
        let backend = TestBackend::new(110, 42);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        let mut server = ProcessSnapshot::fixture("vite", 5173, 32_000_000);
        server.uid = rustix::process::getuid().as_raw();
        server.executable = Some(PathBuf::from("/usr/bin/node"));
        let developer = vec![DeveloperClassification {
            identity: server.identity,
            kind: DeveloperKind::ListeningServer,
            endpoints: vec!["TCP port 5173".to_owned()],
            evidence: vec!["owns 1 TCP listening socket".to_owned()],
        }];
        let mut app = App::new();
        app.apply_scan_batch(ScanBatch {
            processes: vec![server],
            graphical: Vec::new(),
            developer,
            system: SystemMetrics::default(),
        });
        app.handle_key(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::NONE));

        terminal
            .draw(|frame| {
                render(
                    frame,
                    &app,
                    RenderOptions {
                        ascii: true,
                        no_color: true,
                    },
                );
            })
            .expect("render developer layer");
        let rendered = terminal.backend().to_string();
        assert!(rendered.contains("Dev 1"));
        assert!(rendered.contains("vite"));
        assert!(rendered.contains("protected targets are excluded"));

        app.focus = FocusColumn::Details;
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        terminal
            .draw(|frame| {
                render(
                    frame,
                    &app,
                    RenderOptions {
                        ascii: true,
                        no_color: true,
                    },
                );
            })
            .expect("render developer details");
        let rendered = terminal.backend().to_string();
        assert!(rendered.contains("DETECTION"));
        assert!(rendered.contains("TCP port 5173"));
    }

    #[test]
    fn details_replace_the_table_and_show_risk_analysis() {
        let backend = TestBackend::new(110, 32);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        let mut app = App::fixture();
        app.all_processes[0].executable = Some("/usr/bin/nira".into());
        app.focus = FocusColumn::Details;
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

        terminal
            .draw(|frame| {
                render(
                    frame,
                    &app,
                    RenderOptions {
                        ascii: false,
                        no_color: true,
                    },
                );
            })
            .expect("render details");

        let rendered = terminal.backend().to_string();
        assert!(rendered.contains("PROCESS TREE"));
        assert!(rendered.contains("APPLICATION"));
        assert!(rendered.contains("30s trend"));
        assert!(rendered.contains("SAFETY"));
        assert!(rendered.contains("Save your work and close the app normally first."));
        assert!(!rendered.contains("PROCESS NAME"));
    }

    #[test]
    fn force_stop_confirmation_names_the_exact_target_and_risk() {
        let backend = TestBackend::new(110, 24);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        let mut app = App::fixture();
        let identity = app.processes[0].identity;
        let uid = rustix::process::getuid().as_raw();
        app.processes[0].uid = uid;
        app.all_processes[0].uid = uid;
        app.focus = FocusColumn::Details;
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Char('K'), KeyModifiers::SHIFT));
        assert_eq!(app.view, AppView::Confirm);

        terminal
            .draw(|frame| {
                render(
                    frame,
                    &app,
                    RenderOptions {
                        ascii: false,
                        no_color: true,
                    },
                );
            })
            .expect("render confirmation");

        let rendered = terminal.backend().to_string();
        assert!(rendered.contains("CONFIRM FORCE STOP"));
        assert!(rendered.contains(&format!("PID        {}", identity.pid)));
        assert!(rendered.contains("exact PID/start-time identity"));
        assert!(rendered.contains("SIGKILL gives the process no chance"));
        assert!(rendered.contains("ENTER / Y CONFIRM"));
    }

    #[test]
    fn restart_confirmation_explains_direct_exec_limitations() {
        let backend = TestBackend::new(110, 24);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        let mut app = App::fixture();
        let uid = rustix::process::getuid().as_raw();
        app.processes[0].uid = uid;
        app.processes[0].executable = Some(PathBuf::from("/usr/bin/sleep"));
        app.processes[0].cwd = Some(PathBuf::from("/tmp"));
        app.processes[0].command = vec![OsString::from("sleep"), OsString::from("30")];
        app.all_processes[0] = app.processes[0].clone();
        app.focus = FocusColumn::Restart;
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.view, AppView::RestartConfirm);

        terminal
            .draw(|frame| {
                render(
                    frame,
                    &app,
                    RenderOptions {
                        ascii: false,
                        no_color: true,
                    },
                );
            })
            .expect("render restart confirmation");

        let rendered = terminal.backend().to_string();
        assert!(rendered.contains("Restart nira"));
        assert!(rendered.contains("STEPS"));
        assert!(rendered.contains("full environment cannot be restored"));
        assert!(rendered.contains("never force kills during restart"));
    }

    #[test]
    fn action_history_replaces_the_table_and_shows_identity_and_result() {
        let backend = TestBackend::new(100, 18);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        let mut app = App::fixture();
        let identity = app.processes[0].identity;
        app.history.record(
            "nira".to_owned(),
            identity,
            "STOP (SIGTERM)",
            "EXITED; CHILDREN REMAIN — 18423",
        );
        app.handle_key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE));

        terminal
            .draw(|frame| {
                render(
                    frame,
                    &app,
                    RenderOptions {
                        ascii: true,
                        no_color: true,
                    },
                );
            })
            .expect("render action history");

        let rendered = terminal.backend().to_string();
        assert!(rendered.contains("PIDRA ACTION HISTORY"));
        assert!(rendered.contains("STOP (SIGTERM)"));
        assert!(rendered.contains("CHILDREN REMAIN"));
        assert!(rendered.contains(&format!(
            "PID {} / {}",
            identity.pid, identity.start_time_ticks
        )));
        assert!(!rendered.contains("PROCESS NAME"));
    }

    #[test]
    fn help_replaces_the_table_and_states_the_safety_contract() {
        let backend = TestBackend::new(100, 22);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        let mut app = App::fixture();
        app.handle_key(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::SHIFT));

        terminal
            .draw(|frame| {
                render(
                    frame,
                    &app,
                    RenderOptions {
                        ascii: true,
                        no_color: true,
                    },
                );
            })
            .expect("render help");

        let rendered = terminal.backend().to_string();
        assert!(rendered.contains("PIDRA HELP"));
        assert!(rendered.contains("PID plus process start time"));
        assert!(rendered.contains("never escalate automatically"));
        assert!(!rendered.contains("PROCESS NAME"));
    }

    #[test]
    fn ascii_no_color_details_expose_frozen_as_text() {
        let backend = TestBackend::new(100, 26);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        let mut app = App::fixture();
        app.processes[0].state = crate::process::ProcessState::Stopped;
        app.all_processes[0].state = crate::process::ProcessState::Stopped;
        app.focus = FocusColumn::Details;
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

        terminal
            .draw(|frame| {
                render(
                    frame,
                    &app,
                    RenderOptions {
                        ascii: true,
                        no_color: true,
                    },
                );
            })
            .expect("render frozen details");

        let rendered = terminal.backend().to_string();
        assert!(rendered.contains("FROZEN"));
        assert!(rendered.contains("F Resume"));
    }

    #[test]
    fn details_and_restart_scroll_without_hiding_navigation_or_setting_a_background() {
        use ratatui::style::Color;
        for width in [24, 44, 100] {
            let mut terminal = Terminal::new(TestBackend::new(width, 16)).unwrap();
            let options = RenderOptions {
                ascii: true,
                no_color: false,
            };
            let mut app = App::fixture();
            let process = &mut app.all_processes[0];
            process.uid = rustix::process::getuid().as_raw();
            process.executable = Some(PathBuf::from("/usr/bin/sleep"));
            process.cwd = Some(PathBuf::from("/tmp"));
            process.command = vec![OsString::from("sleep"), OsString::from("30")];
            app.processes[0] = process.clone();
            app.focus = FocusColumn::Details;
            app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
            terminal.draw(|frame| render(frame, &app, options)).unwrap();
            assert!(terminal.backend().to_string().contains("APPLICATION"));
            app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
            app.handle_key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE));
            terminal.draw(|frame| render(frame, &app, options)).unwrap();
            let text = terminal.backend().to_string();
            assert!(text.contains("Last action:"));
            assert!(text.contains("Esc Back"));
            assert!(app.info_scroll.get() > 0);
            let end = app.info_scroll.get();
            app.handle_key(KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE));
            assert!(app.info_scroll.get() < end);
            app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
            assert!(!app.details_technical);
            assert_eq!(app.info_scroll.get(), 0);
            app.handle_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE));
            assert_eq!(app.view, AppView::RestartConfirm);
            terminal.draw(|frame| render(frame, &app, options)).unwrap();
            assert!(terminal.backend().to_string().contains("Cancel"));
            app.handle_key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE));
            terminal.draw(|frame| render(frame, &app, options)).unwrap();
            assert!(terminal.backend().to_string().contains("Restart"));
            assert_eq!(terminal.backend().buffer()[(0, 0)].bg, Color::Reset);
            app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
            assert_eq!(app.view, AppView::Details);
            assert_eq!(app.take_control_requests().count(), 0);
            assert_eq!(app.take_restart_requests().count(), 0);
        }
    }

    #[test]
    fn tiny_viewports_render_without_panicking() {
        let backend = TestBackend::new(20, 5);
        let mut terminal = Terminal::new(backend).expect("tiny test terminal");
        let mut app = App::fixture();

        terminal
            .draw(|frame| {
                render(
                    frame,
                    &app,
                    RenderOptions {
                        ascii: true,
                        no_color: true,
                    },
                );
            })
            .expect("render tiny table");
        app.handle_key(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::SHIFT));
        terminal
            .draw(|frame| {
                render(
                    frame,
                    &app,
                    RenderOptions {
                        ascii: true,
                        no_color: true,
                    },
                );
            })
            .expect("render tiny help");
    }
}
