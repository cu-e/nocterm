//! Searching the scrollback in slices that yield to the UI.

use super::*;

impl Terminal {
    pub fn find(&self) -> &FindState {
        &self.find
    }

    pub fn begin_find(
        &mut self,
        query: String,
        direction: SearchDirection,
        anchor: Option<SearchPoint>,
        cx: &mut Context<Self>,
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

    pub fn set_find_options(&mut self, options: SearchOptions, cx: &mut Context<Self>) {
        if self.find.options == options {
            return;
        }
        self.find.options = options;
        let anchor = self.find.result.active.map(|found| found.start);
        self.scan_find(SearchDirection::Stay, anchor, Duration::ZERO, cx);
    }

    pub fn find_next(&mut self, previous: bool, cx: &mut Context<Self>) {
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

    pub fn clear_find(&mut self, cx: &mut Context<Self>) {
        self.find_task = None;
        self.find = FindState {
            options: self.find.options,
            ..Default::default()
        };
        self.emulator.clear_search();
        self.emit_output(cx);
    }

    pub(super) fn refresh_find(&mut self, cx: &mut Context<Self>) {
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

    #[expect(clippy::too_many_lines, reason = "predates the limit")]
    pub(super) fn scan_find(
        &mut self,
        direction: SearchDirection,
        anchor: Option<SearchPoint>,
        delay: Duration,
        cx: &mut Context<Self>,
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
        let mut scan = match self.emulator.search_with_options(
            &self.find.query,
            self.find.options,
            direction,
            anchor,
        ) {
            Ok(scan) => scan,
            Err(error) => {
                self.find.error = Some(error);
                self.find.searching = false;
                self.emit_output(cx);
                return;
            }
        };
        self.find.searching = true;
        self.emit_output(cx);
        self.find_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(delay).await;
            let mut restart = delay != Duration::ZERO;
            let mut previewed = None;
            loop {
                let progress = this.update(cx, |this, cx| {
                    if restart {
                        let Ok(fresh) = this.emulator.search_with_options(
                            &this.find.query,
                            this.find.options,
                            direction,
                            anchor,
                        ) else {
                            return SearchProgress::Invalidated;
                        };
                        scan = fresh;
                        restart = false;
                        previewed = None;
                    }
                    let progress = scan.step(&this.emulator, 4_096);
                    if matches!(progress, SearchProgress::Searching)
                        && let Some(found) = scan.provisional()
                        && previewed != Some(found)
                    {
                        previewed = Some(found);
                        this.find.result.active = Some(found);
                        this.emulator.show_search_match(found);
                        this.emit_output(cx);
                    }
                    if let SearchProgress::Failed(error) = &progress {
                        this.find.error = Some(error.clone());
                        this.find.result = SearchResult::default();
                        this.find.searching = false;
                        this.find_task = None;
                        this.emulator.clear_search();
                        this.emit_output(cx);
                    }
                    if let SearchProgress::Complete(result) = progress {
                        this.find.result = result;
                        this.find.searching = false;
                        this.find_task = None;
                        if let Some(active) = result.active
                            && previewed != Some(active)
                        {
                            this.emulator.show_search_match(active);
                        }
                        this.emit_output(cx);
                    }
                    progress
                });
                match progress {
                    Ok(SearchProgress::Work(work)) => {
                        let result = cx
                            .background_executor()
                            .spawn(async move { work.run() })
                            .await;
                        let accepted =
                            this.update(cx, |this, _| scan.accept_work(&this.emulator, result));
                        if matches!(accepted, Ok(SearchProgress::Invalidated)) {
                            restart = true;
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
                        restart = true;
                        cx.background_executor()
                            .timer(Duration::from_millis(100))
                            .await;
                    }
                    _ => break,
                }
            }
        }));
    }
}
