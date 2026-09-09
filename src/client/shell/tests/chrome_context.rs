use super::*;

#[test]
fn tab_overflow_controls_scroll_the_client_owned_tab_bar() {
    let mut snapshot = snapshot();
    snapshot.tabs.extend((2..=8).map(|number| ClientShellTab {
        tab_id: format!("tab_{number}"),
        workspace_id: "ws_1".into(),
        number,
        label: number.to_string(),
        custom_label: false,
        zoomed: false,
        focused: false,
        agent_status: AgentStatus::Idle,
    }));
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot));
    state.set_pane_surface(surface());
    state.compose(80, 20).expect("overflow tab bar");

    assert!(state.hits.tab_scroll_right.width > 0);
    let scroll_right = state.hits.tab_scroll_right;
    let outcome =
        state.handle_raw_events(vec![RawInputEvent::Mouse(crossterm::event::MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: scroll_right.x + 1,
            row: scroll_right.y,
            modifiers: KeyModifiers::empty(),
        })]);
    assert!(outcome.repaint);
    assert_eq!(state.tab_scroll, 1);

    let mut update = state.snapshot.as_deref().expect("snapshot").clone();
    update.focused_tab_id = Some("tab_8".into());
    for tab in &mut update.tabs {
        tab.focused = tab.tab_id == "tab_8";
    }
    state.set_snapshot(Box::new(update));
    state.compose(80, 20).expect("focused overflow tab");
    assert!(state.hits.tabs.iter().any(|(_, tab_id)| tab_id == "tab_8"));

    state.compose(300, 20).expect("tabs without overflow");
    assert_eq!(state.tab_scroll, 0);
    assert_eq!(state.hits.tabs.len(), 8);
    state.compose(80, 20).expect("focused tab after narrowing");
    assert!(state.hits.tabs.iter().any(|(_, tab_id)| tab_id == "tab_8"));
}

#[test]
fn focused_workspace_change_reveals_new_workspace_in_full_sidebar() {
    let mut initial = snapshot();
    let template = initial.workspaces[0].clone();
    initial.workspaces = (1..=12)
        .map(|number| ClientShellWorkspace {
            workspace_id: format!("ws_{number}"),
            number,
            label: format!("space-{number}"),
            branch: None,
            focused: number == 1,
            ..template.clone()
        })
        .collect();

    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(initial));
    state.set_pane_surface(surface());
    state.compose(106, 20).expect("full sidebar");
    assert!(state.hits.workspace_max_scroll > 0);
    assert!(state
        .hits
        .workspaces
        .iter()
        .all(|hit| hit.workspace_id != "ws_12"));

    let mut update = state.snapshot.as_deref().expect("snapshot").clone();
    update.revision = 2;
    update.focused_workspace_id = Some("ws_12".into());
    for workspace in &mut update.workspaces {
        workspace.focused = workspace.workspace_id == "ws_12";
    }
    let mut updated_surface = surface();
    updated_surface.projection_revision = 2;
    state.set_snapshot(Box::new(update));
    state.set_pane_surface(updated_surface);
    state.compose(106, 2).expect("zero-height workspace body");
    assert!(state.reveal_focused_workspace);
    state.compose(106, 20).expect("updated full sidebar");

    assert!(state
        .hits
        .workspaces
        .iter()
        .any(|hit| hit.workspace_id == "ws_12"));
}

#[test]
fn client_owned_sidebar_dividers_resize_live() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot()));
    state.set_pane_surface(surface());
    state.compose(106, 30).expect("expanded sidebar");
    let workspace_body = state.hits.workspace_body;
    let needless_scroll =
        state.handle_raw_events(vec![RawInputEvent::Mouse(crossterm::event::MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: workspace_body.x,
            row: workspace_body.y,
            modifiers: KeyModifiers::empty(),
        })]);
    assert_eq!(state.hits.workspace_max_scroll, 0);
    assert_eq!(state.workspace_scroll, 0);
    assert!(!needless_scroll.repaint);
    let width_divider = state.hits.sidebar_divider;
    state.handle_raw_events(vec![RawInputEvent::Mouse(crossterm::event::MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: width_divider.x,
        row: width_divider.y + 2,
        modifiers: KeyModifiers::empty(),
    })]);
    let resize =
        state.handle_raw_events(vec![RawInputEvent::Mouse(crossterm::event::MouseEvent {
            kind: MouseEventKind::Drag(MouseButton::Left),
            column: 31,
            row: width_divider.y + 2,
            modifiers: KeyModifiers::empty(),
        })]);
    assert_eq!(state.sidebar_width, 32);
    assert!(state.sidebar_width_manual);
    assert!(resize.repaint);
    assert!(resize.resize);
    state.handle_raw_events(vec![RawInputEvent::Mouse(crossterm::event::MouseEvent {
        kind: MouseEventKind::Up(MouseButton::Left),
        column: 31,
        row: width_divider.y + 2,
        modifiers: KeyModifiers::empty(),
    })]);

    state.set_pane_surface(surface());
    state.compose(106, 30).expect("resized sidebar");
    let section_divider = state.hits.sidebar_section_divider;
    state.handle_raw_events(vec![RawInputEvent::Mouse(crossterm::event::MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: section_divider.x + 2,
        row: section_divider.y,
        modifiers: KeyModifiers::empty(),
    })]);
    let split = state.handle_raw_events(vec![RawInputEvent::Mouse(crossterm::event::MouseEvent {
        kind: MouseEventKind::Drag(MouseButton::Left),
        column: section_divider.x + 2,
        row: 20,
        modifiers: KeyModifiers::empty(),
    })]);
    assert!(state.sidebar_section_split > 0.6);
    assert!(split.repaint);
    assert!(!split.resize);
}

#[test]
fn context_menus_capture_stable_targets_and_route_actions() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot()));
    state.set_pane_surface(surface());
    state.compose(106, 20).expect("composed frame");

    let workspace = state.hits.workspaces[0].rect;
    let open_workspace_menu =
        state.handle_raw_events(vec![RawInputEvent::Mouse(crossterm::event::MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Right),
            column: workspace.x + 2,
            row: workspace.y,
            modifiers: KeyModifiers::empty(),
        })]);
    assert!(open_workspace_menu.actions.is_empty());
    assert!(matches!(
        state.overlay,
        Some(ClientShellOverlay::ContextMenu(ClientContextMenuOverlay {
            target: ClientContextMenuTarget::Workspace { ref workspace_id, .. },
            ..
        })) if workspace_id == "ws_1"
    ));
    let workspace_items = match state.overlay.as_ref() {
        Some(ClientShellOverlay::ContextMenu(menu)) => menu.items(),
        _ => panic!("workspace context menu"),
    };
    assert!(workspace_items
        .iter()
        .any(|item| item.action == ClientContextMenuAction::NewWorktree));
    state.compose(106, 20).expect("workspace context menu");
    let rename = state.hits.context_menu_rows[0].0;
    state.handle_raw_events(vec![RawInputEvent::Mouse(crossterm::event::MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: rename.x + 1,
        row: rename.y,
        modifiers: KeyModifiers::empty(),
    })]);
    assert!(matches!(
        state.overlay,
        Some(ClientShellOverlay::Rename(ClientRenameOverlay {
            target: ClientRenameTarget::Workspace { ref workspace_id },
            ..
        })) if workspace_id == "ws_1"
    ));

    state.overlay = None;
    state.compose(106, 20).expect("composed frame");
    let pane = state.hits.panes[0].rect;
    state.handle_raw_events(vec![RawInputEvent::Mouse(crossterm::event::MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Right),
        column: pane.x + 1,
        row: pane.y,
        modifiers: KeyModifiers::empty(),
    })]);
    state.compose(106, 20).expect("pane context menu");
    let split_index = match state.overlay.as_ref() {
        Some(ClientShellOverlay::ContextMenu(menu)) => menu
            .items()
            .iter()
            .position(|item| item.action == ClientContextMenuAction::SplitRight)
            .expect("split right item"),
        _ => panic!("pane context menu"),
    };
    let split = state.hits.context_menu_rows[split_index].0;
    let outcome =
        state.handle_raw_events(vec![RawInputEvent::Mouse(crossterm::event::MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: split.x + 1,
            row: split.y,
            modifiers: KeyModifiers::empty(),
        })]);
    let [ClientShellAction::Endpoint { request, .. }] = &outcome.actions[..] else {
        panic!("pane split context action should use endpoint API");
    };
    assert!(matches!(
        &request.method,
        crate::api::schema::Method::PaneSplit(params)
            if params.target_pane_id.as_deref() == Some("pane_1")
                && params.direction == crate::api::schema::SplitDirection::Right
    ));
}

#[test]
fn global_menu_opens_from_sidebar_and_routes_client_actions() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot()));
    state.set_pane_surface(surface());
    state.compose(106, 30).expect("shell frame");
    let launcher = state.hits.global_launcher;
    assert_ne!(launcher, Rect::default());

    let open = state.handle_raw_events(vec![RawInputEvent::Mouse(crossterm::event::MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: launcher.x,
        row: launcher.y,
        modifiers: KeyModifiers::empty(),
    })]);
    assert!(open.repaint);
    let menu = state.compose(106, 30).expect("global menu");
    let text = menu
        .cells
        .chunks(menu.width as usize)
        .map(|row| {
            row.iter()
                .map(|cell| cell.symbol.as_str())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("settings"));
    assert!(text.contains("keybinds"));
    assert!(text.contains("reload config"));
    assert!(text.contains("detach"));

    let keybinds = state.hits.global_menu_rows[1].0;
    let help = state.handle_raw_events(vec![RawInputEvent::Mouse(crossterm::event::MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: keybinds.x,
        row: keybinds.y,
        modifiers: KeyModifiers::empty(),
    })]);
    assert!(help.actions.is_empty());
    assert!(matches!(state.overlay, Some(ClientShellOverlay::Help(_))));

    state.overlay = Some(ClientShellOverlay::GlobalMenu(ClientGlobalMenuOverlay {
        highlighted: 3,
    }));
    let detach = state.handle_input_bytes(b"\r");
    assert!(detach.detach);
    assert!(state.overlay.is_none());
}

#[test]
fn new_tab_overlay_owns_text_cursor_and_submits_public_api_request() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot()));
    state.set_pane_surface(surface());
    let mut open = ClientShellInput::default();
    state.record_binding(
        crate::input::KeybindMatch::Action(crate::input::KeybindAction::NewTab),
        &mut open,
    );
    assert!(open.actions.is_empty());
    let frame = state.compose(106, 20).expect("new tab overlay");
    let text = frame
        .cells
        .chunks(frame.width as usize)
        .map(|row| {
            row.iter()
                .map(|cell| cell.symbol.as_str())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("new tab"));
    assert!(text.contains("save"));
    let restored = frame.to_ratatui_buffer().expect("overlay frame");
    assert!(!restored
        .cell((26, 7))
        .expect("overlay title cell")
        .modifier
        .contains(Modifier::DIM));
    assert!(frame.cursor.as_ref().is_some_and(|cursor| cursor.visible));

    assert!(state.handle_input_bytes(b"logs").actions.is_empty());
    let create = state.handle_input_bytes(b"\r");
    let [ClientShellAction::Endpoint { request, .. }] = &create.actions[..] else {
        panic!("new tab save should use endpoint API");
    };
    assert!(matches!(
        &request.method,
        crate::api::schema::Method::TabCreate(params)
            if params.workspace_id.as_deref() == Some("ws_1")
                && params.label.as_deref() == Some("logs")
    ));
    assert!(state.overlay.is_none());
}

#[test]
fn close_confirmation_error_becomes_client_owned_overlay_and_stable_group_close() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot()));
    state.set_pane_surface(surface());
    let mut close = ClientShellInput::default();
    state.record_binding(
        crate::input::KeybindMatch::Action(crate::input::KeybindAction::ClosePane),
        &mut close,
    );
    let [ClientShellAction::Endpoint { request, .. }] = &close.actions[..] else {
        panic!("pane close should use endpoint API");
    };
    let request_id = request.id.clone();
    assert!(
        state
            .handle_endpoint_result(
                "boot-1",
                &request_id,
                Err(ClientShellEndpointError {
                    code: Some("confirmation_required".into()),
                    message: "confirmation required".into(),
                }),
            )
            .0
    );
    let frame = state.compose(106, 20).expect("confirmation overlay");
    let text = frame
        .cells
        .chunks(frame.width as usize)
        .map(|row| {
            row.iter()
                .map(|cell| cell.symbol.as_str())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("Close workspace?"));
    assert!(text.contains("1 pane"));

    let confirm = state.handle_input_bytes(b"\r");
    let [ClientShellAction::Endpoint { request, .. }] = &confirm.actions[..] else {
        panic!("confirmation should use endpoint API");
    };
    assert!(matches!(
        &request.method,
        crate::api::schema::Method::WorkspaceClose(params)
            if params.workspace_id == "ws_1" && params.close_group
    ));
}

fn three_tab_state(focused: &str) -> ClientShellState {
    three_tab_state_with_config(focused, Config::default())
}

fn three_tab_state_with_config(focused: &str, config: Config) -> ClientShellState {
    let mut snapshot = snapshot();
    snapshot.tabs.extend((2..=3).map(|number| ClientShellTab {
        tab_id: format!("tab_{number}"),
        workspace_id: "ws_1".into(),
        number,
        label: number.to_string(),
        custom_label: false,
        zoomed: false,
        focused: false,
        agent_status: AgentStatus::Idle,
    }));
    snapshot.focused_tab_id = Some(focused.into());
    for tab in &mut snapshot.tabs {
        tab.focused = tab.tab_id == focused;
    }
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&config));
    state.set_snapshot(Box::new(snapshot));
    state.set_pane_surface(surface());
    state.compose(106, 20).expect("three tab bar");
    state
}

fn wheel_at(state: &mut ClientShellState, kind: MouseEventKind, column: u16, row: u16) {
    state.handle_raw_events(vec![RawInputEvent::Mouse(MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::empty(),
    })]);
}

fn wheel_over_tab(
    state: &mut ClientShellState,
    kind: MouseEventKind,
    count: u32,
) -> ClientShellInput {
    let focused = state
        .snapshot
        .as_deref()
        .and_then(|snapshot| snapshot.focused_tab_id.clone())
        .expect("focused tab");
    let tab = state
        .hits
        .tabs
        .iter()
        .find(|(_, tab_id)| *tab_id == focused)
        .map(|(rect, _)| *rect)
        .expect("focused tab hit");
    state.handle_raw_events(
        (0..count)
            .map(|_| {
                RawInputEvent::Mouse(MouseEvent {
                    kind,
                    column: tab.x,
                    row: tab.y,
                    modifiers: KeyModifiers::empty(),
                })
            })
            .collect(),
    )
}

fn focused_tab_request(outcome: &ClientShellInput) -> Option<String> {
    outcome.actions.iter().find_map(|action| match action {
        ClientShellAction::Endpoint { request, .. } => match &request.method {
            crate::api::schema::Method::TabFocus(target) => Some(target.tab_id.clone()),
            _ => None,
        },
        _ => None,
    })
}

#[test]
fn horizontal_wheel_swipes_toward_the_neighbor_in_that_direction() {
    let threshold = crate::client::shell::tab_swipe::TAB_SWIPE_THRESHOLD;
    let mut state = three_tab_state("tab_2");
    let short = wheel_over_tab(&mut state, MouseEventKind::ScrollRight, threshold - 1);
    assert!(short.repaint);
    assert_eq!(focused_tab_request(&short), None);
    let commit = wheel_over_tab(&mut state, MouseEventKind::ScrollRight, 1);
    assert_eq!(focused_tab_request(&commit).as_deref(), Some("tab_3"));

    let mut state = three_tab_state("tab_2");
    let commit = wheel_over_tab(&mut state, MouseEventKind::ScrollLeft, threshold);
    assert_eq!(focused_tab_request(&commit).as_deref(), Some("tab_1"));
}

#[test]
fn swiping_past_either_end_wraps_to_the_other() {
    let threshold = crate::client::shell::tab_swipe::TAB_SWIPE_THRESHOLD;
    let mut state = three_tab_state("tab_3");
    let outcome = wheel_over_tab(&mut state, MouseEventKind::ScrollRight, threshold);
    assert_eq!(focused_tab_request(&outcome).as_deref(), Some("tab_1"));
    let mut state = three_tab_state("tab_1");
    let outcome = wheel_over_tab(&mut state, MouseEventKind::ScrollLeft, threshold);
    assert_eq!(focused_tab_request(&outcome).as_deref(), Some("tab_3"));
}

#[test]
fn wrapping_swipe_drains_the_origin_and_fills_the_far_tab_from_its_edge() {
    let threshold = crate::client::shell::tab_swipe::TAB_SWIPE_THRESHOLD;
    let mut state = three_tab_state("tab_1");
    let origin = state.hits.tabs[0].0;
    let last = state.hits.tabs[2].0;
    let accent = state.config.palette.accent;
    wheel_over_tab(&mut state, MouseEventKind::ScrollLeft, threshold / 2);
    let frame = state.compose(106, 20).expect("mid-wrap tab bar");
    let buffer = frame.to_ratatui_buffer().expect("frame buffer");
    let bg_at = |x: u16| buffer.cell((x, origin.y)).expect("tab cell").bg;
    assert_eq!(bg_at(origin.x), accent, "origin keeps its left half");
    assert_ne!(
        bg_at(origin.right() - 1),
        accent,
        "origin has drained on the right"
    );
    assert_eq!(
        bg_at(last.right() - 1),
        accent,
        "last tab fills from its right edge"
    );
    assert_ne!(bg_at(last.x), accent, "last tab's left half is still empty");
}

#[test]
fn one_wheel_burst_switches_at_most_one_tab() {
    let threshold = crate::client::shell::tab_swipe::TAB_SWIPE_THRESHOLD;
    let mut state = three_tab_state("tab_1");
    let outcome = wheel_over_tab(&mut state, MouseEventKind::ScrollRight, threshold * 4);
    let switches = outcome
        .actions
        .iter()
        .filter(|action| {
            matches!(
                action,
                ClientShellAction::Endpoint { request, .. }
                    if matches!(request.method, crate::api::schema::Method::TabFocus(_))
            )
        })
        .count();
    assert_eq!(switches, 1);
    assert_eq!(focused_tab_request(&outcome).as_deref(), Some("tab_2"));

    // Once the burst goes quiet the next swipe starts fresh from the new tab.
    let quiet = std::time::Instant::now()
        + crate::client::shell::tab_swipe::TAB_SWIPE_SETTLE
        + crate::client::shell::tab_swipe::TAB_SWIPE_IDLE;
    assert!(state.tick_tab_swipe(quiet));
    assert!(state.tab_swipe.is_none());
}

#[test]
fn partial_swipe_slides_the_fill_and_snaps_back_when_idle() {
    let threshold = crate::client::shell::tab_swipe::TAB_SWIPE_THRESHOLD;
    let mut state = three_tab_state("tab_1");
    let origin = state.hits.tabs[0].0;
    let accent = state.config.palette.accent;
    let bg_at = |frame: &FrameData, x: u16| {
        frame
            .to_ratatui_buffer()
            .expect("frame buffer")
            .cell((x, origin.y))
            .expect("tab cell")
            .bg
    };
    wheel_over_tab(&mut state, MouseEventKind::ScrollRight, threshold / 2);
    let frame = state.compose(106, 20).expect("mid-swipe tab bar");
    assert_ne!(
        bg_at(&frame, origin.x),
        accent,
        "fill has left the origin edge"
    );
    assert_eq!(
        bg_at(&frame, origin.right() - 1),
        accent,
        "fill still covers the origin"
    );

    let idle_at = std::time::Instant::now() + crate::client::shell::tab_swipe::TAB_SWIPE_IDLE;
    state.tick_tab_swipe(idle_at);
    assert!(state.tick_tab_swipe(idle_at + crate::client::shell::tab_swipe::TAB_SWIPE_SETTLE));
    assert!(state.tab_swipe.is_none());
    let frame = state.compose(106, 20).expect("settled tab bar");
    assert_eq!(bg_at(&frame, origin.x), accent);
}

#[test]
fn the_whole_tab_row_is_swipeable_including_gaps_and_empty_space() {
    let mut state = three_tab_state("tab_1");
    let bar = state.hits.tab_bar;
    let first = state.hits.tabs[0].0;
    assert!(bar.width > 0);
    let gap = first.right();
    assert!(!state
        .hits
        .tabs
        .iter()
        .any(|(rect, _)| rect.x <= gap && gap < rect.right()));
    wheel_at(&mut state, MouseEventKind::ScrollRight, gap, bar.y);
    assert!(state.tab_swipe.is_some(), "gap between tabs swipes");

    let mut state = three_tab_state("tab_1");
    let empty = state.hits.new_tab.right() + 3;
    assert!(empty < bar.right());
    wheel_at(&mut state, MouseEventKind::ScrollRight, empty, bar.y);
    assert!(state.tab_swipe.is_some(), "empty space on the row swipes");
}

#[test]
fn extra_rows_accept_horizontal_swipes_but_leave_vertical_scroll_to_the_pane() {
    let mut state = three_tab_state("tab_1");
    let below = state.hits.tab_bar.bottom();
    wheel_at(&mut state, MouseEventKind::ScrollRight, 40, below);
    assert!(state.tab_swipe.is_none(), "no extra rows by default");

    let with_extra_rows = || {
        let mut config = Config::default();
        config.ui.tab_swipe_extra_rows = 2;
        three_tab_state_with_config("tab_1", config)
    };
    let mut state = with_extra_rows();
    wheel_at(&mut state, MouseEventKind::ScrollRight, 40, below + 1);
    assert!(state.tab_swipe.is_some(), "second extra row swipes");
    let mut state = with_extra_rows();
    wheel_at(&mut state, MouseEventKind::ScrollRight, 40, below + 2);
    assert!(
        state.tab_swipe.is_none(),
        "third row is outside the extra rows"
    );
    let mut state = with_extra_rows();
    wheel_at(&mut state, MouseEventKind::ScrollDown, 40, below);
    assert!(
        state.tab_swipe.is_none(),
        "vertical wheel in the extra rows is not a swipe"
    );
}
