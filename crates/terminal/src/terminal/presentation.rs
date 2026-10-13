//! Screen, search, synchronized updates and presentation revisions.
use super::*;

pub(super) struct TerminalScreen {
    emulator: Emulator,
    display_revision: u64,
    find: FindState,
    find_task: Option<Task<()>>,
    program_title: Option<String>,
    sync_timer: Option<Task<()>>,
}
impl TerminalScreen {
    pub(super) fn new(options: EmulatorOptions) -> Self {
        Self {
            emulator: Emulator::new(TermSize::default(), options),
            display_revision: 0,
            find: FindState::default(),
            find_task: None,
            program_title: None,
            sync_timer: None,
        }
    }
    pub(super) fn emulator(&self) -> &Emulator {
        &self.emulator
    }
    pub(super) fn revision(&self) -> u64 {
        self.display_revision
    }
    pub(super) fn title(&self) -> Option<&str> {
        self.program_title.as_deref()
    }
    pub(super) fn set_title(&mut self, title: Option<String>) -> bool {
        if self.program_title == title {
            return false;
        }
        self.program_title = title;
        true
    }
    pub(super) fn apply_options(&mut self, options: EmulatorOptions) {
        self.emulator.set_options(options);
    }
    pub(super) fn set_palette(&mut self, palette: Palette) {
        self.emulator.set_palette(palette);
    }
    pub(super) fn advance(&mut self, bytes: &[u8]) -> Vec<Effect> {
        self.emulator.advance(bytes)
    }
    pub(super) fn resize(&mut self, size: TermSize) -> bool {
        if size == self.emulator.size() {
            return false;
        }
        self.emulator.resize(size);
        true
    }
    pub(super) fn stop_sync(&mut self) {
        self.sync_timer = None;
    }
    pub(super) fn emit_output(&mut self, cx: &mut Context<Terminal>) {
        self.display_revision = self.display_revision.wrapping_add(1);
        cx.emit(TerminalEvent::Output);
    }
    pub(super) fn scroll(&mut self, scroll: Scroll, cx: &mut Context<Terminal>) {
        if self.emulator.scroll(scroll) {
            self.emit_output(cx);
        }
    }
    pub(super) fn prepare_input(&mut self, cx: &mut Context<Terminal>) {
        if self.emulator.prepare_input() {
            self.emit_output(cx);
        }
    }
    pub(super) fn complete_native_paste(&mut self, cx: &mut Context<Terminal>) {
        self.emulator.scroll(Scroll::Bottom);
        self.emulator.clear_selection();
        self.emit_output(cx);
    }

    pub(super) fn select_all(&mut self, cx: &mut Context<Terminal>) {
        self.emulator.select_all();
        self.emit_output(cx);
    }
    pub(super) fn clear_selection(&mut self, cx: &mut Context<Terminal>) {
        self.emulator.clear_selection();
        self.emit_output(cx);
    }
    pub(super) fn begin_selection(
        &mut self,
        kind: nocterm_vt::SelectionKind,
        at: nocterm_vt::CellPoint,
        side: nocterm_vt::Side,
        cx: &mut Context<Terminal>,
    ) {
        self.emulator.start_selection(kind, at, side);
        self.emit_output(cx);
    }
    pub(super) fn extend_selection(
        &mut self,
        at: nocterm_vt::CellPoint,
        side: nocterm_vt::Side,
        scroll: i32,
        cx: &mut Context<Terminal>,
    ) -> bool {
        let scrolled = scroll != 0 && self.emulator.scroll(Scroll::Lines(scroll));
        self.emulator.update_selection(at, side);
        self.emit_output(cx);
        scrolled
    }
    pub(super) fn schedule_sync(&mut self, cx: &mut Context<Terminal>) {
        if self.sync_timer.is_some() {
            return;
        }
        let Some(deadline) = self.emulator.sync_deadline() else {
            return;
        };
        let wait = deadline.saturating_duration_since(Instant::now());
        self.sync_timer = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(wait).await;
            let _ = this.update(cx, |this, cx| this.finish_sync(cx));
        }));
    }
    pub(super) fn finish_sync(&mut self) -> Option<Vec<Effect>> {
        self.sync_timer = None;
        match self.emulator.sync_deadline() {
            Some(deadline) if deadline <= Instant::now() => Some(self.emulator.finish_sync()),
            _ => None,
        }
    }
}
impl TerminalScreen {
    pub(super) fn find(&self) -> &FindState {
        &self.find
    }

    pub(super) fn begin_find(
        &mut self,
        query: String,
        direction: SearchDirection,
        anchor: Option<SearchPoint>,
        cx: &mut Context<Terminal>,
    ) {
        self.find_task = None;
        self.emulator.clear_search();
        self.find = FindState {
            options: self.find.options,
            query: query
                .chars()
                .take(nocterm_vt::MAX_SEARCH_QUERY + 1)
                .collect(),
            ..Default::default()
        };
        self.scan_find(direction, anchor, Duration::ZERO, cx);
    }

    pub(super) fn set_find_options(&mut self, options: SearchOptions, cx: &mut Context<Terminal>) {
        if self.find.options == options {
            return;
        }
        self.find.options = options;
        let anchor = self.find.result.active.map(|found| found.start);
        self.scan_find(SearchDirection::Stay, anchor, Duration::ZERO, cx);
    }

    pub(super) fn find_next(&mut self, previous: bool, cx: &mut Context<Terminal>) {
        let anchor = self.find.result.active.map(|m| m.start);
        self.scan_find(
            if previous {
                SearchDirection::Previous
            } else {
                SearchDirection::Next
            },
            anchor,
            Duration::ZERO,
            cx,
        );
    }

    pub(super) fn clear_find(&mut self, cx: &mut Context<Terminal>) {
        self.find_task = None;
        self.find = FindState {
            options: self.find.options,
            ..Default::default()
        };
        self.emulator.clear_search();
        self.emit_output(cx);
    }

    pub(super) fn refresh_find(&mut self, cx: &mut Context<Terminal>) {
        if self.find.query.is_empty() {
            return;
        }
        // A running task owns its restart throttle. Continuous output must not
        // keep postponing the start of every attempt indefinitely.
        if self.find.searching {
            return;
        }
        let anchor = self.find.result.active.map(|m| m.start);
        self.scan_find(
            SearchDirection::Stay,
            anchor,
            Duration::from_millis(100),
            cx,
        );
    }

    pub(super) fn scan_find(
        &mut self,
        direction: SearchDirection,
        anchor: Option<SearchPoint>,
        delay: Duration,
        cx: &mut Context<Terminal>,
    ) {
        self.find_task = None;
        self.find.error = None;
        self.find.result = SearchResult::default();
        self.emulator.clear_search();
        if self.find.query.is_empty() {
            self.find.searching = false;
            self.find.result = SearchResult::default();
            self.emit_output(cx);
            return;
        }
        let scan = match self.emulator.search_with_options(
            &self.find.query,
            self.find.options,
            direction,
            anchor,
        ) {
            Ok(scan) => scan,
            Err(error) => {
                self.find.error = Some(error.to_string());
                self.find.searching = false;
                self.emit_output(cx);
                return;
            }
        };
        self.find.searching = true;
        self.emit_output(cx);
        self.find_task = Some(
            SearchRun {
                scan,
                direction,
                anchor,
                restart: delay != Duration::ZERO,
                previewed: None,
            }
            .spawn(delay, cx),
        );
    }

    fn step_find(&mut self, run: &mut SearchRun, cx: &mut Context<Terminal>) -> SearchProgress {
        if run.restart {
            let Ok(fresh) = self.emulator.search_with_options(
                &self.find.query,
                self.find.options,
                run.direction,
                run.anchor,
            ) else {
                return SearchProgress::Invalidated;
            };
            run.scan = fresh;
            run.restart = false;
            run.previewed = None;
        }
        let progress = run.scan.step(&self.emulator, 4_096);
        if matches!(progress, SearchProgress::Searching)
            && let Some(found) = run.scan.provisional()
            && run.previewed != Some(found)
        {
            run.previewed = Some(found);
            self.find.result.active = Some(found);
            self.emulator.show_search_match(found);
            self.emit_output(cx);
        }
        if let SearchProgress::Failed(error) = &progress {
            self.find.error = Some(error.to_string());
            self.find.result = SearchResult::default();
            self.find.searching = false;
            self.find_task = None;
            self.emulator.clear_search();
            self.emit_output(cx);
        }
        if let SearchProgress::Complete(result) = progress {
            self.find.result = result;
            self.find.searching = false;
            self.find_task = None;
            if let Some(active) = result.active
                && run.previewed != Some(active)
            {
                self.emulator.show_search_match(active);
            }
            self.emit_output(cx);
        }
        progress
    }
}
struct SearchRun {
    scan: nocterm_vt::SearchScan,
    direction: SearchDirection,
    anchor: Option<SearchPoint>,
    restart: bool,
    previewed: Option<nocterm_vt::SearchMatch>,
}
impl SearchRun {
    fn spawn(mut self, delay: Duration, cx: &mut Context<Terminal>) -> Task<()> {
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(delay).await;
            loop {
                let progress = this.update(cx, |this, cx| this.screen.step_find(&mut self, cx));
                match progress {
                    Ok(SearchProgress::Work(work)) => {
                        let result = cx
                            .background_executor()
                            .spawn(async move { work.run() })
                            .await;
                        let accepted = this
                            .update(cx, |this, _| self.scan.accept_work(this.emulator(), result));
                        if matches!(accepted, Ok(SearchProgress::Invalidated)) {
                            self.restart = true;
                            cx.background_executor()
                                .timer(Duration::from_millis(100))
                                .await;
                        } else if accepted.is_err() {
                            break;
                        }
                        // A batch can contain many short logical lines; no per-line timer.
                    }
                    Ok(SearchProgress::Searching) => {
                        cx.background_executor()
                            .timer(Duration::from_millis(1))
                            .await
                    }
                    Ok(SearchProgress::Invalidated) => {
                        self.restart = true;
                        cx.background_executor()
                            .timer(Duration::from_millis(100))
                            .await;
                    }
                    _ => break,
                }
            }
        })
    }
}
