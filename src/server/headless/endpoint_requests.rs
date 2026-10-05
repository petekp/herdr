use super::*;

impl HeadlessServer {
    pub(super) fn handle_client_shell_endpoint_request(
        &mut self,
        client_id: u64,
        boot_id: String,
        mut request: Box<api::schema::Request>,
    ) -> bool {
        let Some(client) = self.clients.get(&client_id) else {
            return false;
        };
        if !matches!(client.mode, ClientConnectionMode::ClientShell) {
            self.remove_client_and_resize_if_needed(client_id);
            return true;
        }
        let request_id = request.id.clone();
        if !crate::server::client_commands::supports_client_shell_method(&request.method) {
            let message = crate::server::client_commands::error_message(
                boot_id,
                request_id,
                "unsupported_endpoint_command",
                "this method is not available through the client shell command lane",
            );
            self.send_to_client(client_id, message);
            return false;
        }
        if boot_id != self.client_shell_boot_id {
            let message = crate::server::client_commands::error_message(
                boot_id,
                request_id,
                "stale_boot",
                "endpoint command targeted an earlier server boot",
            );
            self.send_to_client(client_id, message);
            return false;
        }
        let surface_active = client.shell_surface_active;
        if let api::schema::Method::ClientShellSurfaceSet(params) = &request.method {
            let Some((changed, projection_revision)) =
                self.set_client_shell_surface_active(client_id, params.active)
            else {
                return false;
            };
            self.send_to_client(
                client_id,
                crate::server::client_commands::success_message_with_result(
                    boot_id,
                    request_id,
                    api::schema::ResponseResult::ClientShellSurfaceSet {
                        active: params.active,
                        projection_revision,
                    },
                ),
            );
            return changed;
        }
        if let api::schema::Method::ClientShellSurfaceRead(target) = &request.method {
            let tab_id = target.tab_id.clone();
            let (message, changed) = match self.read_client_shell_surface(client_id, &tab_id) {
                Ok((result, changed)) => (
                    crate::server::client_commands::success_message_with_result(
                        boot_id, request_id, result,
                    ),
                    changed,
                ),
                Err((code, message)) => (
                    crate::server::client_commands::error_message(
                        boot_id, request_id, code, message,
                    ),
                    false,
                ),
            };
            self.send_to_client(client_id, message);
            return changed;
        }
        if client.shell_endpoint_command_in_flight {
            let message = crate::server::client_commands::error_message(
                boot_id,
                request_id,
                "endpoint_busy",
                "this endpoint is still processing another command",
            );
            self.send_to_client(client_id, message);
            return false;
        }
        if !surface_active {
            let message = crate::server::client_commands::error_message(
                boot_id,
                request_id,
                "surface_inactive",
                "this method requires an active client shell surface",
            );
            self.send_to_client(client_id, message);
            return false;
        }

        let api_request_id = format!(
            "endpoint:{}:{client_id}:{request_id}",
            self.client_shell_boot_id
        );
        request.id = api_request_id.clone();
        let (respond_to, response_rx) = std::sync::mpsc::channel();
        if let Err(err) = crate::server::client_commands::spawn_response_waiter(
            client_id,
            boot_id.clone(),
            request_id.clone(),
            response_rx,
            self.server_event_tx.clone(),
        ) {
            let message = crate::server::client_commands::error_message(
                boot_id,
                request_id,
                "server_unavailable",
                format!("failed to start endpoint response bridge: {err}"),
            );
            self.send_to_client(client_id, message);
            return false;
        }
        if let Some(client) = self.clients.get_mut(&client_id) {
            client.shell_endpoint_command_in_flight = true;
            // A later source restore has a new projection revision. Keep this request's lease
            // so a delayed worktree response cannot focus a pane after endpoint switching.
            client.shell_endpoint_command_surface_revision = Some(client.shell_projection_revision);
            let deferred_worktree = matches!(
                &request.method,
                api::schema::Method::WorktreeCreate(_) | api::schema::Method::WorktreeRemove(_)
            );
            let deferred_navigation = matches!(
                &request.method,
                api::schema::Method::WorktreeCreate(params) if params.focus
            );
            client.shell_deferred_navigation_request_id =
                deferred_worktree.then(|| api_request_id.clone());
            client.shell_deferred_navigation_response = deferred_navigation.then(Vec::new);
        }
        let foreground_changed = self.promote_client_to_foreground(client_id);
        foreground_changed
            | self.handle_client_shell_api_request(
                client_id,
                api::ApiRequestMessage {
                    request: *request,
                    respond_to,
                    response_write_complete: None,
                    stream_active: None,
                },
            )
    }

    /// Renders `tab_id` once at the requesting client's surface size without
    /// focusing it. Returns whether another client now needs a full render.
    fn read_client_shell_surface(
        &mut self,
        client_id: u64,
        tab_id: &str,
    ) -> Result<(api::schema::ResponseResult, bool), (&'static str, String)> {
        let Some((workspace_index, tab_index)) = self.app.parse_tab_id(tab_id) else {
            return Err(("tab_not_found", format!("tab {tab_id} not found")));
        };
        let Some(client) = self.clients.get(&client_id) else {
            return Err(("client_missing", "the requesting client is gone".into()));
        };
        let (cols, rows) = client.terminal_size;
        let cell_size = client.cell_size;
        let target = crate::ui::TabSurfaceTarget {
            workspace_index,
            tab_index,
        };
        let (buffer, _, _, _) = crate::server::render_stream::render_tab_surface_virtual(
            &self.app.state,
            &self.app.terminal_runtimes,
            Some(target),
            Rect::new(0, 0, cols, rows),
            false,
            cell_size,
        );
        let frame = FrameData::from_ratatui_buffer_with_hyperlinks(&buffer, None, &[]);
        // Rendering cleared the dirty rows that retained patches for this tab
        // are built from, so any client showing it must take a full frame next.
        let viewing = self
            .clients
            .iter()
            .filter(|(_, client)| client.is_shell_client())
            .map(|(&id, _)| id)
            .filter(|&id| self.shell_target_for_client(id) == Some(target))
            .collect::<Vec<_>>();
        for id in &viewing {
            if let Some(client) = self.clients.get_mut(id) {
                client.defer_full_render();
            }
        }
        Ok((
            api::schema::ResponseResult::ClientShellSurface {
                tab_id: tab_id.to_owned(),
                cols,
                rows,
                lines: crate::server::client_shell::surface_lines(&frame),
            },
            !viewing.is_empty(),
        ))
    }
}
