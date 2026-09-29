use std::{
    cell::Cell,
    collections::{HashMap, HashSet, VecDeque},
    time::Instant,
};

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::process::{
    ApplicationResources, DeveloperClassification, GuiClassification, GuiConfidence,
    ProcessIdentity, ProcessSnapshot, ResourceTrend, ScanBatch, TrendTracker,
    aggregate_application_resources,
    cpu::SystemMetrics,
    tree::{ProcessTree, TreeNode},
};
use crate::{
    control::{
        ControlOutcome, ControlRequest, ControlResult, SignalAction,
        diagnosis::{DiagnosisContext, diagnose_after_signal, diagnose_dispatch_failure},
        restart::{RestartRequest, RestartResult, RestartSource, resolve_restart_source},
        risk::assess_termination,
    },
    history::ActionHistory,
    process::ProcessState,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppView {
    Table,
    Developer,
    Details,
    Confirm,
    RestartConfirm,
    History,
    Help,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Confirmation {
    pub identity: ProcessIdentity,
    pub process_name: String,
    pub action: SignalAction,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestartConfirmation {
    pub identity: ProcessIdentity,
    pub process_name: String,
    pub source: RestartSource,
    pub return_to: AppView,
}

#[derive(Debug)]
struct PendingObservation {
    action: SignalAction,
    started_at: Instant,
    context: DiagnosisContext,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusColumn {
    Restart,
    Stop,
    Details,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortMode {
    Memory,
    Cpu,
    Name,
    Pid,
    WriteRate,
}

impl SortMode {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Memory => "MEMORY",
            Self::Cpu => "CPU",
            Self::Name => "NAME",
            Self::Pid => "PID",
            Self::WriteRate => "WRITE",
        }
    }

    fn next(self) -> Self {
        match self {
            Self::Memory => Self::Cpu,
            Self::Cpu => Self::Name,
            Self::Name => Self::Pid,
            Self::Pid => Self::WriteRate,
            Self::WriteRate => Self::Memory,
        }
    }
}

impl FocusColumn {
    fn label(self) -> &'static str {
        match self {
            Self::Restart => "RESTART",
            Self::Stop => "STOP",
            Self::Details => "DETAILS",
        }
    }

    fn previous(self) -> Self {
        match self {
            Self::Restart => Self::Restart,
            Self::Stop => Self::Restart,
            Self::Details => Self::Stop,
        }
    }

    fn next(self) -> Self {
        match self {
            Self::Restart => Self::Stop,
            Self::Stop => Self::Details,
            Self::Details => Self::Details,
        }
    }
}

#[derive(Debug)]
pub struct App {
    pub processes: Vec<ProcessSnapshot>,
    pub all_processes: Vec<ProcessSnapshot>,
    pub gui_classifications: HashMap<ProcessIdentity, GuiClassification>,
    pub developer_classifications: HashMap<ProcessIdentity, DeveloperClassification>,
    pub system_metrics: SystemMetrics,
    pub resources: HashMap<ProcessIdentity, ApplicationResources>,
    pub sort_mode: SortMode,
    pub selected: usize,
    pub focus: FocusColumn,
    pub status: String,
    pub should_quit: bool,
    pub searching: bool,
    pub search_query: String,
    pub view: AppView,
    pub details_root: Option<ProcessIdentity>,
    pub details_selected: usize,
    pub details_technical: bool,
    pub info_scroll: Cell<u16>,
    pub expanded_nodes: HashSet<ProcessIdentity>,
    pub confirmation: Option<Confirmation>,
    pub restart_confirmation: Option<RestartConfirmation>,
    pub history: ActionHistory,
    history_return: AppView,
    help_return: AppView,
    requested_pid: Option<i32>,
    pending_control: VecDeque<ControlRequest>,
    pending_restarts: VecDeque<RestartRequest>,
    queued_diagnosis: HashMap<(ProcessIdentity, SignalAction), DiagnosisContext>,
    pending_observation: HashMap<ProcessIdentity, PendingObservation>,
    pub latest_actions: HashMap<ProcessIdentity, String>,
    trends: TrendTracker,
    table_view: AppView,
}

impl App {
    #[must_use]
    pub fn graphical_total(&self) -> usize {
        self.gui_classifications.len()
    }

    #[must_use]
    pub fn developer_total(&self) -> usize {
        self.developer_classifications.len()
    }

    #[must_use]
    pub fn developer_layer_active(&self) -> bool {
        self.table_view == AppView::Developer
    }

    pub fn display_name<'a>(&'a self, process: &'a ProcessSnapshot) -> &'a str {
        self.gui_classifications
            .get(&process.identity)
            .and_then(|classification| classification.display_name.as_deref())
            .filter(|name| !name.trim().is_empty())
            .unwrap_or(&process.name)
    }

    #[must_use]
    pub fn restart_source_for(&self, identity: ProcessIdentity) -> RestartSource {
        self.process_by_identity(identity).map_or_else(
            || RestartSource::Unavailable {
                reason: "process identity no longer exists".to_owned(),
            },
            resolve_restart_source,
        )
    }

    #[must_use]
    pub fn new() -> Self {
        Self {
            processes: Vec::new(),
            all_processes: Vec::new(),
            gui_classifications: HashMap::new(),
            developer_classifications: HashMap::new(),
            system_metrics: SystemMetrics::default(),
            resources: HashMap::new(),
            sort_mode: SortMode::Memory,
            selected: 0,
            focus: FocusColumn::Restart,
            status: "Scanning /proc…".to_owned(),
            should_quit: false,
            searching: false,
            search_query: String::new(),
            view: AppView::Table,
            details_root: None,
            details_selected: 0,
            details_technical: false,
            info_scroll: Cell::new(0),
            expanded_nodes: HashSet::new(),
            confirmation: None,
            restart_confirmation: None,
            history: ActionHistory::default(),
            history_return: AppView::Table,
            help_return: AppView::Table,
            requested_pid: None,
            pending_control: VecDeque::new(),
            pending_restarts: VecDeque::new(),
            queued_diagnosis: HashMap::new(),
            pending_observation: HashMap::new(),
            latest_actions: HashMap::new(),
            trends: TrendTracker::default(),
            table_view: AppView::Table,
        }
    }

    #[must_use]
    pub fn with_history(history: ActionHistory) -> Self {
        let mut app = Self::new();
        app.history = history;
        app
    }

    #[must_use]
    pub fn fixture() -> Self {
        let processes = vec![
            ProcessSnapshot::fixture("nira", 18_422, 1_932_735_283),
            ProcessSnapshot::fixture("firefox", 2_204, 3_328_599_654),
            ProcessSnapshot::fixture("qs", 1_198, 432_013_312),
            ProcessSnapshot::fixture("pipewire", 806, 32_505_856),
            ProcessSnapshot::fixture("ffmpeg", 19_102, 0),
        ];
        let gui_classifications = processes
            .iter()
            .map(|process| {
                (
                    process.identity,
                    GuiClassification {
                        identity: process.identity,
                        confidence: GuiConfidence::Probable,
                        display_name: None,
                        application_scope: None,
                        evidence: vec!["Phase 0 fixture".to_owned()],
                    },
                )
            })
            .collect();
        Self {
            all_processes: processes.clone(),
            processes,
            gui_classifications,
            developer_classifications: HashMap::new(),
            system_metrics: SystemMetrics::default(),
            resources: HashMap::new(),
            sort_mode: SortMode::Memory,
            selected: 0,
            focus: FocusColumn::Restart,
            status: "Phase 0 fixture — no real process actions are enabled".to_owned(),
            should_quit: false,
            searching: false,
            search_query: String::new(),
            view: AppView::Table,
            details_root: None,
            details_selected: 0,
            details_technical: false,
            info_scroll: Cell::new(0),
            expanded_nodes: HashSet::new(),
            confirmation: None,
            restart_confirmation: None,
            history: ActionHistory::default(),
            history_return: AppView::Table,
            help_return: AppView::Table,
            requested_pid: None,
            pending_control: VecDeque::new(),
            pending_restarts: VecDeque::new(),
            queued_diagnosis: HashMap::new(),
            pending_observation: HashMap::new(),
            latest_actions: HashMap::new(),
            trends: TrendTracker::default(),
            table_view: AppView::Table,
        }
    }

    pub fn apply_scan_batch(&mut self, batch: ScanBatch) {
        let replace_with_scan_summary = self.status == "Scanning /proc…"
            || self.status.starts_with("Showing ")
            || self.status.starts_with("Process scan failed:");
        let selected_identity = self
            .processes
            .get(self.selected)
            .map(|process| process.identity);
        self.all_processes = batch.processes;
        self.system_metrics = batch.system;
        self.gui_classifications = batch
            .graphical
            .into_iter()
            .map(|classification| (classification.identity, classification))
            .collect();
        let pidra_pid = i32::try_from(std::process::id()).unwrap_or(i32::MAX);
        self.developer_classifications = batch
            .developer
            .into_iter()
            .filter(|classification| {
                self.all_processes
                    .iter()
                    .find(|process| process.identity == classification.identity)
                    .is_some_and(|process| {
                        assess_termination(process, &self.all_processes, pidra_pid).rating
                            != crate::control::risk::RiskRating::Protected
                    })
            })
            .map(|classification| (classification.identity, classification))
            .collect();
        let resource_roots = self
            .gui_classifications
            .keys()
            .chain(self.developer_classifications.keys())
            .copied()
            .collect::<Vec<_>>();
        self.resources = aggregate_application_resources(&self.all_processes, resource_roots);
        self.trends.update(&self.resources);
        self.open_requested_pid();
        self.rebuild_visible(selected_identity);
        self.observe_pending_actions();
        let details_missing = matches!(self.view, AppView::Details | AppView::Confirm)
            && self
                .details_root
                .is_some_and(|identity| self.process_by_identity(identity).is_none());
        if details_missing {
            self.status = "The detailed process identity no longer exists".to_owned();
        } else if matches!(self.view, AppView::Table | AppView::Developer)
            && replace_with_scan_summary
        {
            self.status = self.scan_summary();
        }
        self.clamp_details_selection();
    }

    pub fn request_initial_pid(&mut self, pid: Option<i32>) {
        self.requested_pid = pid.filter(|pid| *pid > 0);
    }

    fn open_requested_pid(&mut self) {
        let Some(pid) = self.requested_pid else {
            return;
        };
        let Some(process) = self
            .all_processes
            .iter()
            .find(|process| process.identity.pid == pid)
        else {
            return;
        };
        self.details_root = Some(process.identity);
        self.details_selected = 0;
        self.details_technical = false;
        self.info_scroll.set(0);
        self.expanded_nodes.clear();
        self.expanded_nodes.insert(process.identity);
        self.view = AppView::Details;
        self.status = format!("Inspecting requested PID {pid}");
        self.requested_pid = None;
    }

    fn rebuild_visible(&mut self, selected_identity: Option<ProcessIdentity>) {
        let previous_index = self.selected;
        let query = self.search_query.to_lowercase();
        let mut processes: Vec<_> = self
            .all_processes
            .iter()
            .filter(|process| {
                if self.table_view == AppView::Developer {
                    self.developer_classifications
                        .contains_key(&process.identity)
                } else {
                    self.gui_classifications.contains_key(&process.identity)
                }
            })
            .filter(|process| {
                query.is_empty()
                    || self.display_name(process).to_lowercase().contains(&query)
                    || process.name.to_lowercase().contains(&query)
                    || process.identity.pid.to_string().contains(&query)
            })
            .cloned()
            .collect();

        processes.sort_by(|left, right| self.compare_processes(left, right));
        self.processes = processes;
        self.selected = selected_identity
            .and_then(|identity| self.index_of(identity))
            .unwrap_or_else(|| previous_index.min(self.processes.len().saturating_sub(1)));
    }

    fn compare_processes(
        &self,
        left: &ProcessSnapshot,
        right: &ProcessSnapshot,
    ) -> std::cmp::Ordering {
        let left_resources = self.application_resources(left.identity);
        let right_resources = self.application_resources(right.identity);
        let primary = match self.sort_mode {
            SortMode::Memory => right_resources
                .preferred_memory_bytes()
                .cmp(&left_resources.preferred_memory_bytes()),
            SortMode::Cpu => right_resources
                .cpu_percent
                .total_cmp(&left_resources.cpu_percent),
            SortMode::Name => self
                .display_name(left)
                .to_lowercase()
                .cmp(&self.display_name(right).to_lowercase()),
            SortMode::Pid => left.identity.pid.cmp(&right.identity.pid),
            SortMode::WriteRate => right_resources
                .write_rate_bytes
                .total_cmp(&left_resources.write_rate_bytes),
        };
        primary
            .then_with(|| left.name.cmp(&right.name))
            .then_with(|| left.identity.pid.cmp(&right.identity.pid))
    }

    #[must_use]
    pub fn application_resources(&self, identity: ProcessIdentity) -> ApplicationResources {
        self.resources.get(&identity).copied().unwrap_or_else(|| {
            self.process_by_identity(identity).map_or_else(
                ApplicationResources::default,
                |process| ApplicationResources {
                    process_count: 1,
                    cpu_percent: process.cpu_percent,
                    rss_bytes: process.rss_bytes,
                    pss_bytes: process.pss_bytes.unwrap_or(0),
                    pss_process_count: usize::from(process.pss_bytes.is_some()),
                    read_rate_bytes: process.read_rate_bytes.unwrap_or(0.0),
                    write_rate_bytes: process.write_rate_bytes.unwrap_or(0.0),
                },
            )
        })
    }

    #[must_use]
    pub fn resource_trend(&self, identity: ProcessIdentity) -> Option<ResourceTrend> {
        self.trends.summary(identity)
    }

    fn cycle_sort_mode(&mut self) {
        let selected_identity = self
            .processes
            .get(self.selected)
            .map(|process| process.identity);
        self.sort_mode = self.sort_mode.next();
        self.rebuild_visible(selected_identity);
        self.status = format!("Sorted by {}", self.sort_mode.label());
    }

    fn scan_summary(&self) -> String {
        if self.table_view == AppView::Developer {
            format!(
                "Showing {} current-user developer/server processes; protected targets are excluded",
                self.processes.len()
            )
        } else {
            format!(
                "Showing {} GUI processes from {} scanned processes",
                self.processes.len(),
                self.all_processes.len()
            )
        }
    }

    fn toggle_developer_layer(&mut self) {
        let selected_identity = self
            .processes
            .get(self.selected)
            .map(|process| process.identity);
        self.table_view = if self.table_view == AppView::Developer {
            AppView::Table
        } else {
            AppView::Developer
        };
        self.view = self.table_view;
        self.selected = 0;
        self.rebuild_visible(selected_identity);
        self.status = self.scan_summary();
    }

    pub fn report_scan_error(&mut self, error: &str) {
        self.status = format!("Process scan failed: {error}");
    }

    fn index_of(&self, identity: ProcessIdentity) -> Option<usize> {
        self.processes
            .iter()
            .position(|process| process.identity == identity)
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        if key.kind == KeyEventKind::Release {
            return;
        }

        if key.modifiers.contains(KeyModifiers::CONTROL)
            && matches!(key.code, KeyCode::Char('c' | 'C'))
        {
            self.should_quit = true;
            return;
        }

        if self.searching {
            self.handle_search_key(key);
            return;
        }

        if matches!(self.view, AppView::Details | AppView::RestartConfirm) {
            match key.code {
                KeyCode::PageDown => {
                    self.info_scroll
                        .set(self.info_scroll.get().saturating_add(5));
                    return;
                }
                KeyCode::PageUp => {
                    self.info_scroll
                        .set(self.info_scroll.get().saturating_sub(5));
                    return;
                }
                KeyCode::Home => {
                    self.info_scroll.set(0);
                    return;
                }
                KeyCode::End => {
                    self.info_scroll.set(u16::MAX);
                    return;
                }
                _ => {}
            }
        }
        match self.view {
            AppView::Details => {
                self.handle_details_key(key);
                return;
            }
            AppView::Confirm => {
                self.handle_confirmation_key(key);
                return;
            }
            AppView::RestartConfirm => {
                self.handle_restart_confirmation_key(key);
                return;
            }
            AppView::History => {
                self.handle_history_key(key);
                return;
            }
            AppView::Help => {
                self.handle_help_key(key);
                return;
            }
            AppView::Table | AppView::Developer => {}
        }

        match key.code {
            KeyCode::Esc if self.view == AppView::Developer => self.toggle_developer_layer(),
            KeyCode::Up => self.select_previous(),
            KeyCode::Down => self.select_next(),
            KeyCode::Left => self.focus = self.focus.previous(),
            KeyCode::Right => self.focus = self.focus.next(),
            KeyCode::Char('r' | 'R') => self.focus = FocusColumn::Restart,
            KeyCode::Char('s' | 'S') => self.focus = FocusColumn::Stop,
            KeyCode::Char('d' | 'D') => self.focus = FocusColumn::Details,
            KeyCode::Enter => self.activate_focused_action(),
            KeyCode::Char('/') => self.searching = true,
            KeyCode::Char('v' | 'V') => self.toggle_developer_layer(),
            KeyCode::Char('o' | 'O') => self.cycle_sort_mode(),
            KeyCode::Char('h' | 'H') => self.open_history(self.table_view),
            KeyCode::Char('?') => self.open_help(self.table_view),
            KeyCode::Char('q' | 'Q') => self.should_quit = true,
            _ => {}
        }
    }

    fn handle_details_key(&mut self, key: KeyEvent) {
        if matches!(
            key.code,
            KeyCode::Up | KeyCode::Down | KeyCode::Left | KeyCode::Right | KeyCode::Tab
        ) {
            self.info_scroll.set(0);
        }
        match key.code {
            KeyCode::Tab => {
                self.details_technical = !self.details_technical;
            }
            KeyCode::Esc => {
                self.view = self.table_view;
                self.status = if self.table_view == AppView::Developer {
                    "Returned to developer/server process layer".to_owned()
                } else {
                    "Returned to GUI process table".to_owned()
                };
            }
            KeyCode::Char('q' | 'Q') => self.should_quit = true,
            KeyCode::Up => self.details_selected = self.details_selected.saturating_sub(1),
            KeyCode::Down => {
                let node_count = self.detail_nodes().len();
                if self.details_selected + 1 < node_count {
                    self.details_selected += 1;
                }
            }
            KeyCode::Right | KeyCode::Enter => self.expand_selected_detail_node(),
            KeyCode::Left => self.collapse_or_select_parent(),
            KeyCode::Char('f' | 'F') => {
                let action = self.selected_detail_process().map(|process| {
                    if process.state == ProcessState::Stopped {
                        SignalAction::Resume
                    } else {
                        SignalAction::Freeze
                    }
                });
                if let Some(action) = action {
                    self.queue_selected_detail_action(action);
                }
            }
            KeyCode::Char('t' | 'T') => self.queue_selected_detail_action(SignalAction::Stop),
            KeyCode::Char('r' | 'R') => self.open_selected_restart_confirmation(AppView::Details),
            KeyCode::Char('h' | 'H') => self.open_history(AppView::Details),
            KeyCode::Char('?') => self.open_help(AppView::Details),
            KeyCode::Char('K') if key.modifiers.contains(KeyModifiers::SHIFT) => {
                self.open_force_stop_confirmation();
            }
            _ => {}
        }
    }

    fn handle_confirmation_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Enter | KeyCode::Char('y' | 'Y') => {
                if let Some(confirmation) = self.confirmation.take() {
                    if let Some(process) = self.process_by_identity(confirmation.identity).cloned()
                    {
                        self.queued_diagnosis.insert(
                            (confirmation.identity, confirmation.action),
                            DiagnosisContext::capture(&process, &self.all_processes),
                        );
                    }
                    self.pending_control.push_back(ControlRequest {
                        identity: confirmation.identity,
                        action: confirmation.action,
                    });
                    self.status = format!(
                        "Queued {} for {} ({})",
                        confirmation.action.label(),
                        confirmation.process_name,
                        confirmation.identity.pid
                    );
                }
                self.view = AppView::Details;
            }
            KeyCode::Esc | KeyCode::Char('n' | 'N') => {
                self.confirmation = None;
                self.view = AppView::Details;
                self.status = "Force Stop cancelled".to_owned();
            }
            _ => {}
        }
    }

    fn handle_restart_confirmation_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Enter | KeyCode::Char('y' | 'Y') => {
                if let Some(confirmation) = self.restart_confirmation.take() {
                    let return_to = confirmation.return_to;
                    self.status = format!(
                        "Queued RESTART for {} ({})",
                        confirmation.process_name, confirmation.identity.pid
                    );
                    tracing::info!(
                        pid = confirmation.identity.pid,
                        start_ticks = confirmation.identity.start_time_ticks,
                        "restart queued after confirmation"
                    );
                    self.pending_restarts.push_back(RestartRequest {
                        identity: confirmation.identity,
                        process_name: confirmation.process_name,
                        source: confirmation.source,
                    });
                    self.view = return_to;
                }
            }
            KeyCode::Esc | KeyCode::Char('n' | 'N') => {
                if let Some(confirmation) = self.restart_confirmation.take() {
                    self.view = confirmation.return_to;
                } else {
                    self.view = AppView::Table;
                }
                self.status = "Restart cancelled".to_owned();
            }
            _ => {}
        }
    }

    fn handle_history_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc | KeyCode::Char('h' | 'H') => {
                self.view = self.history_return;
                self.status = "Closed action history".to_owned();
            }
            KeyCode::Char('q' | 'Q') => self.should_quit = true,
            KeyCode::Char('?') => self.open_help(AppView::History),
            _ => {}
        }
    }

    fn open_history(&mut self, return_to: AppView) {
        self.history_return = return_to;
        self.view = AppView::History;
        self.status = format!("{} completed actions this session", self.history.len());
    }

    fn handle_help_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc | KeyCode::Char('?') => {
                self.view = self.help_return;
                self.status = "Closed help".to_owned();
            }
            KeyCode::Char('q' | 'Q') => self.should_quit = true,
            _ => {}
        }
    }

    fn open_help(&mut self, return_to: AppView) {
        self.help_return = return_to;
        self.view = AppView::Help;
        self.status = "PIDRA keyboard and safety help".to_owned();
    }

    fn handle_search_key(&mut self, key: KeyEvent) {
        let selected_identity = self
            .processes
            .get(self.selected)
            .map(|process| process.identity);
        match key.code {
            KeyCode::Esc | KeyCode::Enter => self.searching = false,
            KeyCode::Backspace => {
                self.search_query.pop();
                self.rebuild_visible(selected_identity);
            }
            KeyCode::Char(character)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.search_query.push(character);
                self.rebuild_visible(selected_identity);
            }
            _ => {}
        }
        self.status = if self.search_query.is_empty() {
            "Search GUI process name or PID".to_owned()
        } else {
            format!(
                "Search /{} — {} matches",
                self.search_query,
                self.processes.len()
            )
        };
    }

    pub fn select_previous(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    pub fn select_next(&mut self) {
        if self.selected + 1 < self.processes.len() {
            self.selected += 1;
        }
    }

    pub fn select_from_pointer(&mut self, row: usize, focus: Option<FocusColumn>) {
        if row >= self.processes.len() {
            return;
        }
        let was_selected = self.selected == row;
        let was_focused = focus.is_some_and(|column| column == self.focus);
        self.selected = row;
        if let Some(column) = focus {
            self.focus = column;
            if was_selected && was_focused {
                self.activate_focused_action();
            } else if let Some(process) = self.processes.get(row) {
                self.status = format!(
                    "Selected {} for {} ({}) — click again or press Enter",
                    column.label(),
                    process.name,
                    process.identity.pid
                );
            }
        } else if let Some(process) = self.processes.get(row) {
            self.status = format!("Selected {} ({})", process.name, process.identity.pid);
        }
    }

    fn activate_focused_action(&mut self) {
        let Some(process) = self.processes.get(self.selected).cloned() else {
            return;
        };

        match self.focus {
            FocusColumn::Restart => {
                self.open_restart_confirmation(&process, self.table_view);
            }
            FocusColumn::Stop => {
                self.queue_action(&process, SignalAction::Stop);
            }
            FocusColumn::Details => {
                self.view = AppView::Details;
                self.details_root = Some(process.identity);
                self.details_selected = 0;
                self.details_technical = false;
                self.info_scroll.set(0);
                self.expanded_nodes.clear();
                self.expanded_nodes.insert(process.identity);
                self.status = format!("Inspecting {} ({})", process.name, process.identity.pid);
            }
        }
    }

    #[must_use]
    pub fn detail_nodes(&self) -> Vec<TreeNode> {
        let Some(root) = self.details_root else {
            return Vec::new();
        };
        ProcessTree::new(&self.all_processes).visible_nodes(root, &self.expanded_nodes)
    }

    #[must_use]
    pub fn selected_detail_process(&self) -> Option<&ProcessSnapshot> {
        let node = self.detail_nodes().get(self.details_selected).copied()?;
        self.process_by_identity(node.identity)
    }

    #[must_use]
    pub fn process_by_identity(&self, identity: ProcessIdentity) -> Option<&ProcessSnapshot> {
        self.all_processes
            .iter()
            .find(|process| process.identity == identity)
    }

    fn expand_selected_detail_node(&mut self) {
        let Some(node) = self.detail_nodes().get(self.details_selected).copied() else {
            return;
        };
        if node.has_children {
            self.expanded_nodes.insert(node.identity);
        } else {
            self.status = "Selected process has no child processes".to_owned();
        }
    }

    fn collapse_or_select_parent(&mut self) {
        let nodes = self.detail_nodes();
        let Some(node) = nodes.get(self.details_selected).copied() else {
            return;
        };
        if node.expanded && node.has_children {
            self.expanded_nodes.remove(&node.identity);
            self.clamp_details_selection();
            return;
        }
        if node.depth == 0 {
            return;
        }
        if let Some(parent_index) = nodes[..self.details_selected]
            .iter()
            .rposition(|candidate| candidate.depth < node.depth)
        {
            self.details_selected = parent_index;
        }
    }

    fn clamp_details_selection(&mut self) {
        self.details_selected = self
            .details_selected
            .min(self.detail_nodes().len().saturating_sub(1));
    }

    fn queue_selected_detail_action(&mut self, action: SignalAction) {
        let Some(process) = self.selected_detail_process().cloned() else {
            self.status = "Selected process identity no longer exists".to_owned();
            return;
        };
        self.queue_action(&process, action);
    }

    fn queue_action(&mut self, process: &ProcessSnapshot, action: SignalAction) {
        let assessment = assess_termination(
            process,
            &self.all_processes,
            i32::try_from(std::process::id()).unwrap_or(i32::MAX),
        );
        if assessment.rating == crate::control::risk::RiskRating::Protected {
            self.status = format!("Action blocked: {}", assessment.evidence.join("; "));
            tracing::warn!(
                pid = process.identity.pid,
                start_ticks = process.identity.start_time_ticks,
                action = action.label(),
                "process action blocked by safety policy"
            );
            return;
        }
        self.pending_control.push_back(ControlRequest {
            identity: process.identity,
            action,
        });
        self.queued_diagnosis.insert(
            (process.identity, action),
            DiagnosisContext::capture(process, &self.all_processes),
        );
        self.status = format!(
            "Queued {} for {} ({})",
            action.label(),
            process.name,
            process.identity.pid
        );
        tracing::info!(
            pid = process.identity.pid,
            start_ticks = process.identity.start_time_ticks,
            action = action.label(),
            "process action queued"
        );
    }

    fn open_force_stop_confirmation(&mut self) {
        let Some(process) = self.selected_detail_process().cloned() else {
            return;
        };
        let assessment = assess_termination(
            &process,
            &self.all_processes,
            i32::try_from(std::process::id()).unwrap_or(i32::MAX),
        );
        if assessment.rating == crate::control::risk::RiskRating::Protected {
            self.status = format!("Force Stop blocked: {}", assessment.evidence.join("; "));
            return;
        }
        self.confirmation = Some(Confirmation {
            identity: process.identity,
            process_name: process.name,
            action: SignalAction::ForceStop,
        });
        self.view = AppView::Confirm;
    }

    fn open_selected_restart_confirmation(&mut self, return_to: AppView) {
        let Some(process) = self.selected_detail_process().cloned() else {
            self.status = "Selected process identity no longer exists".to_owned();
            return;
        };
        self.open_restart_confirmation(&process, return_to);
    }

    fn open_restart_confirmation(&mut self, process: &ProcessSnapshot, return_to: AppView) {
        let assessment = assess_termination(
            process,
            &self.all_processes,
            i32::try_from(std::process::id()).unwrap_or(i32::MAX),
        );
        if assessment.rating == crate::control::risk::RiskRating::Protected {
            self.status = format!("Restart blocked: {}", assessment.evidence.join("; "));
            return;
        }
        let source = self.restart_source_for(process.identity);
        if let RestartSource::Unavailable { reason } = source {
            self.status = format!(
                "Restart unavailable for {} ({}): {reason}",
                process.name, process.identity.pid
            );
            return;
        }
        self.info_scroll.set(0);
        self.restart_confirmation = Some(RestartConfirmation {
            identity: process.identity,
            process_name: self.display_name(process).to_owned(),
            source,
            return_to,
        });
        self.view = AppView::RestartConfirm;
    }

    pub fn take_control_requests(&mut self) -> impl Iterator<Item = ControlRequest> + '_ {
        self.pending_control.drain(..)
    }

    pub fn take_restart_requests(&mut self) -> impl Iterator<Item = RestartRequest> + '_ {
        self.pending_restarts.drain(..)
    }

    pub fn report_control_dispatch_error(&mut self, error: &str) {
        self.status = format!("Control worker error: {error}");
        tracing::error!(error, "control worker dispatch failed");
    }

    pub fn report_restart_dispatch_error(&mut self, error: &str) {
        self.status = format!("Restart worker error: {error}");
        tracing::error!(error, "restart worker dispatch failed");
    }

    pub fn apply_restart_result(&mut self, result: RestartResult) {
        let message = format!("RESTART: {}", result.outcome.message());
        self.latest_actions
            .insert(result.request.identity, message.clone());
        self.history.record(
            result.request.process_name,
            result.request.identity,
            "RESTART",
            result.outcome.message(),
        );
        self.status = message;
        tracing::info!(
            pid = result.request.identity.pid,
            start_ticks = result.request.identity.start_time_ticks,
            outcome = %result.outcome.message(),
            "restart completed"
        );
    }

    pub fn apply_control_result(&mut self, result: ControlResult) {
        let message = format!(
            "{}: {}",
            result.request.action.label(),
            result.outcome.message()
        );
        self.latest_actions
            .insert(result.request.identity, message.clone());
        self.status = message;
        tracing::info!(
            pid = result.request.identity.pid,
            start_ticks = result.request.identity.start_time_ticks,
            action = result.request.action.label(),
            outcome = %result.outcome.message(),
            "kernel signal result received"
        );
        let context = self
            .queued_diagnosis
            .remove(&(result.request.identity, result.request.action));
        if matches!(result.outcome, ControlOutcome::Sent(_)) {
            let context = context.or_else(|| {
                self.process_by_identity(result.request.identity)
                    .map(|process| DiagnosisContext::capture(process, &self.all_processes))
            });
            if let Some(context) = context {
                self.pending_observation.insert(
                    result.request.identity,
                    PendingObservation {
                        action: result.request.action,
                        started_at: Instant::now(),
                        context,
                    },
                );
            }
        } else {
            if let Some(diagnosis) = diagnose_dispatch_failure(&result.outcome) {
                let message = format!("{}: {}", result.request.action.label(), diagnosis.summary());
                self.latest_actions
                    .insert(result.request.identity, message.clone());
                self.history.record(
                    context.map_or_else(
                        || format!("PID {}", result.request.identity.pid),
                        |context| context.target.name,
                    ),
                    result.request.identity,
                    result.request.action.label(),
                    diagnosis.summary(),
                );
                self.status = message;
            }
        }
    }

    fn observe_pending_actions(&mut self) {
        let now = Instant::now();
        let mut finished = Vec::new();
        for (identity, pending) in &self.pending_observation {
            if let Some(diagnoses) = diagnose_after_signal(
                &pending.context,
                pending.action,
                &self.all_processes,
                now.saturating_duration_since(pending.started_at),
            ) {
                let summary = diagnoses
                    .iter()
                    .map(|diagnosis| diagnosis.summary())
                    .collect::<Vec<_>>()
                    .join("; ");
                finished.push((
                    *identity,
                    pending.context.target.name.clone(),
                    pending.action,
                    summary,
                ));
            }
        }
        for (identity, process_name, action, summary) in finished {
            self.pending_observation.remove(&identity);
            let message = format!("{}: {summary}", action.label());
            self.latest_actions.insert(identity, message.clone());
            tracing::info!(
                pid = identity.pid,
                start_ticks = identity.start_time_ticks,
                action = action.label(),
                diagnosis = %summary,
                "process action observation completed"
            );
            self.history
                .record(process_name, identity, action.label(), summary);
            self.status = message;
        }
    }

    #[must_use]
    pub fn latest_action_for(&self, identity: ProcessIdentity) -> Option<&str> {
        self.latest_actions.get(&identity).map(String::as_str)
    }
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use std::{ffi::OsString, path::PathBuf};

    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    use crate::control::{ControlOutcome, ControlResult, SignalAction};
    use crate::process::{
        DeveloperClassification, DeveloperKind, GuiClassification, GuiConfidence, ProcessSnapshot,
        ScanBatch, cpu::SystemMetrics,
    };

    use super::{App, AppView, FocusColumn, SortMode};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn open_fixture_details(app: &mut App) {
        let identity = app.processes[0].identity;
        let uid = rustix::process::getuid().as_raw();
        app.processes
            .iter_mut()
            .find(|process| process.identity == identity)
            .expect("visible fixture process")
            .uid = uid;
        app.all_processes
            .iter_mut()
            .find(|process| process.identity == identity)
            .expect("fixture process")
            .uid = uid;
        app.focus = FocusColumn::Details;
        app.handle_key(key(KeyCode::Enter));
        assert_eq!(app.view, AppView::Details);
    }

    #[test]
    fn moves_rows_without_leaving_bounds() {
        let mut app = App::fixture();

        app.handle_key(key(KeyCode::Up));
        assert_eq!(app.selected, 0);
        app.handle_key(key(KeyCode::Down));
        assert_eq!(app.selected, 1);

        for _ in 0..20 {
            app.handle_key(key(KeyCode::Down));
        }
        assert_eq!(app.selected, app.processes.len() - 1);
    }

    #[test]
    fn changes_action_focus() {
        let mut app = App::fixture();

        app.handle_key(key(KeyCode::Right));
        assert_eq!(app.focus, FocusColumn::Stop);
        app.handle_key(key(KeyCode::Right));
        assert_eq!(app.focus, FocusColumn::Details);
        app.handle_key(key(KeyCode::Left));
        assert_eq!(app.focus, FocusColumn::Stop);
    }

    #[test]
    fn letter_shortcuts_focus_without_activating() {
        let mut app = App::fixture();
        let original_status = app.status.clone();

        app.handle_key(key(KeyCode::Char('d')));

        assert_eq!(app.focus, FocusColumn::Details);
        assert_eq!(app.status, original_status);
    }

    #[test]
    fn quit_key_requests_exit() {
        let mut app = App::fixture();

        app.handle_key(key(KeyCode::Char('q')));

        assert!(app.should_quit);
    }

    #[test]
    fn refresh_preserves_selection_by_identity() {
        let mut app = App::fixture();
        app.selected = 2;
        let selected_identity = app.processes[2].identity;
        let mut refreshed = app.processes.clone();
        refreshed[2].rss_bytes = u64::MAX;

        let graphical = app.gui_classifications.values().cloned().collect();
        app.apply_scan_batch(ScanBatch {
            processes: refreshed,
            graphical,
            developer: Vec::new(),
            system: SystemMetrics::default(),
        });

        assert_eq!(app.processes[app.selected].identity, selected_identity);
    }

    #[test]
    fn sort_modes_preserve_identity_and_use_tree_resources() {
        let mut root = ProcessSnapshot::fixture("z-root", 400, 100);
        let mut child = ProcessSnapshot::fixture("child", 401, 900);
        child.parent_pid = Some(root.identity.pid);
        root.cpu_percent = 1.0;
        child.cpu_percent = 40.0;
        let other = ProcessSnapshot::fixture("a-other", 300, 500);
        let graphical = [&root, &other]
            .into_iter()
            .map(|process| GuiClassification {
                identity: process.identity,
                confidence: GuiConfidence::Probable,
                display_name: None,
                application_scope: None,
                evidence: vec!["test".to_owned()],
            })
            .collect();
        let mut app = App::new();
        app.apply_scan_batch(ScanBatch {
            processes: vec![root.clone(), child, other],
            graphical,
            developer: Vec::new(),
            system: SystemMetrics::default(),
        });

        assert_eq!(app.processes[0].identity, root.identity);
        assert_eq!(app.application_resources(root.identity).rss_bytes, 1_000);
        let selected = app.processes[0].identity;
        app.handle_key(key(KeyCode::Char('o')));
        assert_eq!(app.sort_mode, SortMode::Cpu);
        assert_eq!(app.processes[app.selected].identity, selected);
        app.handle_key(key(KeyCode::Char('o')));
        assert_eq!(app.sort_mode, SortMode::Name);
        assert_eq!(app.processes[0].name, "a-other");
        assert_eq!(app.processes[app.selected].identity, selected);
    }

    #[test]
    fn search_filters_name_and_pid_without_quitting_on_q() {
        let mut app = App::fixture();
        app.handle_key(key(KeyCode::Char('/')));
        app.handle_key(key(KeyCode::Char('q')));

        assert!(app.searching);
        assert!(!app.should_quit);
        assert_eq!(app.processes.len(), 1);
        assert_eq!(app.processes[0].name, "qs");

        app.handle_key(key(KeyCode::Backspace));
        for character in "2204".chars() {
            app.handle_key(key(KeyCode::Char(character)));
        }
        assert_eq!(app.processes.len(), 1);
        assert_eq!(app.processes[0].name, "firefox");
    }

    #[test]
    fn developer_layer_shows_only_safe_classified_developer_processes() {
        let mut gui = ProcessSnapshot::fixture("firefox", 301, 100);
        gui.uid = rustix::process::getuid().as_raw();
        gui.executable = Some(PathBuf::from("/usr/bin/firefox"));
        let mut server = ProcessSnapshot::fixture("vite", 302, 50);
        server.uid = rustix::process::getuid().as_raw();
        server.executable = Some(PathBuf::from("/usr/bin/node"));
        let graphical = vec![GuiClassification {
            identity: gui.identity,
            confidence: GuiConfidence::Confirmed,
            display_name: None,
            application_scope: None,
            evidence: vec!["window".to_owned()],
        }];
        let developer = vec![DeveloperClassification {
            identity: server.identity,
            kind: DeveloperKind::ListeningServer,
            endpoints: vec!["TCP port 5173".to_owned()],
            evidence: vec!["owns a listener".to_owned()],
        }];
        let mut app = App::new();
        app.apply_scan_batch(ScanBatch {
            processes: vec![gui, server],
            graphical,
            developer,
            system: SystemMetrics::default(),
        });

        assert_eq!(app.processes.len(), 1);
        assert_eq!(app.processes[0].name, "firefox");
        app.handle_key(key(KeyCode::Char('v')));
        assert_eq!(app.view, AppView::Developer);
        assert_eq!(app.processes.len(), 1);
        assert_eq!(app.processes[0].name, "vite");
        assert!(app.status.contains("protected targets are excluded"));

        app.handle_key(key(KeyCode::Esc));
        assert_eq!(app.view, AppView::Table);
        assert_eq!(app.processes[0].name, "firefox");
    }

    #[test]
    fn developer_layer_actions_keep_the_exact_process_identity() {
        let mut server = ProcessSnapshot::fixture("uvicorn", 303, 50);
        server.uid = rustix::process::getuid().as_raw();
        server.executable = Some(PathBuf::from("/usr/bin/uvicorn"));
        let expected = server.identity;
        let developer = vec![DeveloperClassification {
            identity: expected,
            kind: DeveloperKind::ListeningServer,
            endpoints: vec!["TCP port 8000".to_owned()],
            evidence: vec!["owns a listener".to_owned()],
        }];
        let mut app = App::new();
        app.apply_scan_batch(ScanBatch {
            processes: vec![server],
            graphical: Vec::new(),
            developer,
            system: SystemMetrics::default(),
        });
        app.handle_key(key(KeyCode::Char('v')));
        app.focus = FocusColumn::Stop;

        app.handle_key(key(KeyCode::Enter));

        let requests: Vec<_> = app.take_control_requests().collect();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].identity, expected);
        assert_eq!(requests[0].action, SignalAction::Stop);
    }

    #[test]
    fn ten_thousand_processes_remain_navigable() {
        let processes: Vec<_> = (1..=10_000)
            .map(|pid| ProcessSnapshot::fixture(&format!("app-{pid}"), pid, pid as u64))
            .collect();
        let graphical = processes
            .iter()
            .map(|process| GuiClassification {
                identity: process.identity,
                confidence: GuiConfidence::Probable,
                display_name: None,
                application_scope: None,
                evidence: vec!["load fixture".to_owned()],
            })
            .collect();
        let mut app = App::new();
        app.apply_scan_batch(ScanBatch {
            processes,
            graphical,
            developer: Vec::new(),
            system: SystemMetrics::default(),
        });

        for _ in 0..9_999 {
            app.select_next();
        }

        assert_eq!(app.selected, 9_999);
        assert_eq!(app.processes.len(), 10_000);
    }

    #[test]
    fn second_click_on_selected_action_uses_keyboard_command() {
        let mut app = App::fixture();

        app.select_from_pointer(0, Some(FocusColumn::Details));
        assert_eq!(app.view, AppView::Table);
        app.select_from_pointer(0, Some(FocusColumn::Details));

        assert_eq!(app.view, AppView::Details);
        assert!(app.status.contains("Inspecting nira"));
    }

    #[test]
    fn details_expand_children_and_return_to_table() {
        let mut app = App::fixture();
        let mut child = ProcessSnapshot::fixture("renderer", 18_423, 512);
        child.parent_pid = Some(18_422);
        app.all_processes.push(child);
        app.focus = FocusColumn::Details;

        app.handle_key(key(KeyCode::Enter));
        assert_eq!(app.view, AppView::Details);
        assert_eq!(app.detail_nodes().len(), 2);
        app.handle_key(key(KeyCode::Down));
        assert_eq!(
            app.selected_detail_process().map(|p| p.name.as_str()),
            Some("renderer")
        );
        app.handle_key(key(KeyCode::Left));
        assert_eq!(app.details_selected, 0);
        app.handle_key(key(KeyCode::Esc));
        assert_eq!(app.view, AppView::Table);
    }

    #[test]
    fn table_stop_queues_sigterm_for_the_exact_identity() {
        let mut app = App::fixture();
        let expected = app.processes[0].identity;
        let uid = rustix::process::getuid().as_raw();
        app.processes[0].uid = uid;
        app.all_processes[0].uid = uid;
        app.focus = FocusColumn::Stop;

        app.handle_key(key(KeyCode::Enter));

        let requests: Vec<_> = app.take_control_requests().collect();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].identity, expected);
        assert_eq!(requests[0].action, SignalAction::Stop);
    }

    #[test]
    fn force_stop_requires_explicit_confirmation_and_can_be_cancelled() {
        let mut app = App::fixture();
        open_fixture_details(&mut app);
        let expected = app
            .selected_detail_process()
            .expect("detail target")
            .identity;

        app.handle_key(KeyEvent::new(KeyCode::Char('K'), KeyModifiers::SHIFT));

        assert_eq!(app.view, AppView::Confirm);
        let confirmation = app.confirmation.as_ref().expect("confirmation");
        assert_eq!(confirmation.identity, expected);
        assert_eq!(confirmation.action, SignalAction::ForceStop);
        app.handle_key(key(KeyCode::Char('n')));
        assert_eq!(app.view, AppView::Details);
        assert!(app.confirmation.is_none());
        assert_eq!(app.take_control_requests().count(), 0);
    }

    #[test]
    fn force_stop_confirmation_queues_sigkill_only_after_acceptance() {
        let mut app = App::fixture();
        open_fixture_details(&mut app);
        let expected = app
            .selected_detail_process()
            .expect("detail target")
            .identity;
        app.handle_key(KeyEvent::new(KeyCode::Char('K'), KeyModifiers::SHIFT));

        app.handle_key(key(KeyCode::Char('y')));

        assert_eq!(app.view, AppView::Details);
        let requests: Vec<_> = app.take_control_requests().collect();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].identity, expected);
        assert_eq!(requests[0].action, SignalAction::ForceStop);
    }

    #[test]
    fn sent_signal_is_reported_as_pending_kernel_observation() {
        let mut app = App::fixture();
        let identity = app.processes[0].identity;
        app.apply_control_result(ControlResult {
            request: crate::control::ControlRequest {
                identity,
                action: SignalAction::Stop,
            },
            outcome: ControlOutcome::Sent(crate::control::DeliveryMethod::Pidfd),
        });

        assert!(
            app.latest_action_for(identity)
                .is_some_and(|message| message.contains("pidfd"))
        );
    }

    fn prepare_direct_restart(app: &mut App) {
        let identity = app.processes[0].identity;
        let uid = rustix::process::getuid().as_raw();
        for process in app
            .processes
            .iter_mut()
            .chain(app.all_processes.iter_mut())
            .filter(|process| process.identity == identity)
        {
            process.uid = uid;
            process.executable = Some(PathBuf::from("/usr/bin/sleep"));
            process.cwd = Some(PathBuf::from("/tmp"));
            process.command = vec![OsString::from("sleep"), OsString::from("30")];
        }
    }

    #[test]
    fn restart_requires_confirmation_and_cancel_queues_nothing() {
        let mut app = App::fixture();
        prepare_direct_restart(&mut app);
        app.focus = FocusColumn::Restart;

        app.handle_key(key(KeyCode::Enter));

        assert_eq!(app.view, AppView::RestartConfirm);
        assert!(app.restart_confirmation.is_some());
        app.handle_key(key(KeyCode::Char('q')));
        assert!(!app.should_quit);
        assert_eq!(app.view, AppView::RestartConfirm);
        app.handle_key(key(KeyCode::Char('n')));
        assert_eq!(app.view, AppView::Table);
        assert_eq!(app.take_restart_requests().count(), 0);
    }

    #[test]
    fn accepted_restart_keeps_the_resolved_source_and_identity() {
        let mut app = App::fixture();
        prepare_direct_restart(&mut app);
        let expected = app.processes[0].identity;
        app.focus = FocusColumn::Restart;
        app.handle_key(key(KeyCode::Enter));

        app.handle_key(key(KeyCode::Char('y')));

        let requests: Vec<_> = app.take_restart_requests().collect();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].identity, expected);
        assert!(matches!(
            requests[0].source,
            crate::control::restart::RestartSource::Direct { .. }
        ));
    }

    fn dispatch_fixture_stop(app: &mut App) -> crate::control::ControlRequest {
        let identity = app.processes[0].identity;
        let uid = rustix::process::getuid().as_raw();
        for process in app
            .processes
            .iter_mut()
            .chain(app.all_processes.iter_mut())
            .filter(|process| process.identity == identity)
        {
            process.uid = uid;
            process.executable = Some(PathBuf::from("/usr/bin/demo"));
            process.cgroups = vec!["0::/app.slice/demo.service".to_owned()];
        }
        app.focus = FocusColumn::Stop;
        app.handle_key(key(KeyCode::Enter));
        app.take_control_requests().next().expect("stop request")
    }

    #[test]
    fn scanner_update_diagnoses_a_supervisor_restart_and_records_history() {
        let mut app = App::fixture();
        let request = dispatch_fixture_stop(&mut app);
        app.apply_control_result(ControlResult {
            request,
            outcome: ControlOutcome::Sent(crate::control::DeliveryMethod::Pidfd),
        });
        let mut replacement = ProcessSnapshot::fixture("renamed-demo", 29_001, 1);
        replacement.uid = rustix::process::getuid().as_raw();
        replacement.identity.start_time_ticks = request.identity.start_time_ticks + 10;
        replacement.cgroups = vec!["0::/app.slice/demo.service".to_owned()];

        app.apply_scan_batch(ScanBatch {
            processes: vec![replacement.clone()],
            graphical: Vec::new(),
            developer: Vec::new(),
            system: SystemMetrics::default(),
        });

        let result = app.latest_action_for(request.identity).expect("diagnosis");
        assert!(result.contains("RESTARTED"));
        assert!(result.contains(&replacement.identity.pid.to_string()));
        assert_eq!(app.history.len(), 1);
    }

    #[test]
    fn scanner_update_names_a_surviving_captured_child() {
        let mut app = App::fixture();
        let mut child = ProcessSnapshot::fixture("survivor", 18_423, 1);
        child.parent_pid = Some(app.processes[0].identity.pid);
        child.uid = rustix::process::getuid().as_raw();
        app.all_processes.push(child.clone());
        let request = dispatch_fixture_stop(&mut app);
        app.apply_control_result(ControlResult {
            request,
            outcome: ControlOutcome::Sent(crate::control::DeliveryMethod::Pidfd),
        });

        app.apply_scan_batch(ScanBatch {
            processes: vec![child.clone()],
            graphical: Vec::new(),
            developer: Vec::new(),
            system: SystemMetrics::default(),
        });

        let result = app.latest_action_for(request.identity).expect("diagnosis");
        assert!(result.contains("CHILDREN REMAIN"));
        assert!(result.contains(&child.identity.pid.to_string()));
    }

    #[test]
    fn history_is_keyboard_reachable_and_returns_to_the_previous_view() {
        let mut app = App::fixture();
        app.history.record(
            "demo".to_owned(),
            app.processes[0].identity,
            "STOP",
            "EXITED",
        );

        app.handle_key(key(KeyCode::Char('h')));
        assert_eq!(app.view, AppView::History);
        app.handle_key(key(KeyCode::Esc));
        assert_eq!(app.view, AppView::Table);
    }

    #[test]
    fn requested_pid_opens_details_after_the_first_matching_scan() {
        let fixture = App::fixture();
        let target = fixture.all_processes[1].identity;
        let graphical = fixture.gui_classifications.values().cloned().collect();
        let mut app = App::new();
        app.request_initial_pid(Some(target.pid));

        app.apply_scan_batch(ScanBatch {
            processes: fixture.all_processes,
            graphical,
            developer: Vec::new(),
            system: SystemMetrics::default(),
        });

        assert_eq!(app.view, AppView::Details);
        assert_eq!(app.details_root, Some(target));
    }

    #[test]
    fn help_is_keyboard_reachable_and_returns_to_details() {
        let mut app = App::fixture();
        app.focus = FocusColumn::Details;
        app.handle_key(key(KeyCode::Enter));
        app.handle_key(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::SHIFT));
        assert_eq!(app.view, AppView::Help);

        app.handle_key(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::SHIFT));
        assert_eq!(app.view, AppView::Details);
    }

    #[test]
    fn scanner_refresh_does_not_erase_a_control_result() {
        let mut app = App::fixture();
        let identity = app.processes[0].identity;
        app.apply_control_result(ControlResult {
            request: crate::control::ControlRequest {
                identity,
                action: SignalAction::Stop,
            },
            outcome: ControlOutcome::PermissionDenied,
        });
        let expected = app.status.clone();
        let graphical = app.gui_classifications.values().cloned().collect();

        app.apply_scan_batch(ScanBatch {
            processes: app.all_processes.clone(),
            graphical,
            developer: Vec::new(),
            system: SystemMetrics::default(),
        });

        assert_eq!(app.status, expected);
        assert!(app.status.contains("PERMISSION DENIED"));
    }

    #[test]
    fn first_action_click_explains_confirmation_and_second_click_activates() {
        let mut app = App::fixture();
        let uid = rustix::process::getuid().as_raw();
        app.processes[0].uid = uid;
        app.all_processes[0].uid = uid;

        app.select_from_pointer(0, Some(FocusColumn::Stop));
        assert!(app.status.contains("click again or press Enter"));
        assert_eq!(app.take_control_requests().count(), 0);

        app.select_from_pointer(0, Some(FocusColumn::Stop));
        let requests: Vec<_> = app.take_control_requests().collect();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].action, SignalAction::Stop);
    }
}
