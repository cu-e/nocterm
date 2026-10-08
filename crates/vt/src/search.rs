//! Cancellable grid search. Literal matching streams; regex work uses bounded batches.
use crate::Emulator;
use alacritty_terminal::{
    grid::Dimensions as _,
    index::{Column, Line},
    term::cell::Flags,
};
use std::{
    collections::{HashMap, VecDeque},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

/// Maximum query length in Unicode scalar values.
pub const MAX_SEARCH_QUERY: usize = 2_048;
/// Regexes operate on complete logical lines, joining soft wraps only.
pub const MAX_REGEX_LINE_BYTES: usize = 64 * 1024;
const MAX_REGEX_BATCH_BYTES: usize = 64 * 1024;
const MAX_REGEX_COMPILED_BYTES: usize = 256 * 1024;
const MAX_REGEX_WORK_TIME: Duration = Duration::from_millis(250);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SearchOptions {
    pub regex: bool,
    pub case_sensitive: bool,
    pub whole_word: bool,
}
impl Default for SearchOptions {
    fn default() -> Self {
        Self {
            regex: false,
            case_sensitive: true,
            whole_word: false,
        }
    }
}
/// Grid position, including a combining-scalar offset on the cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct SearchPoint {
    pub line: i32,
    pub column: u16,
    pub(crate) part: usize,
}
/// Inclusive cell range; search never replaces the user selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SearchMatch {
    pub start: SearchPoint,
    pub end: SearchPoint,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchDirection {
    Next,
    Previous,
    Stay,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SearchResult {
    pub active: Option<SearchMatch>,
    pub ordinal: u64,
    pub count: u64,
}
#[derive(Debug, PartialEq, Eq)]
pub enum SearchProgress {
    Searching,
    Invalidated,
    Complete(SearchResult),
    Failed(String),
    /// Run this bounded work on a background executor, then call accept_work.
    Work(RegexWork),
}
#[derive(Default, Debug)]
struct Summary {
    first: Option<SearchMatch>,
    last: Option<SearchMatch>,
    best: Option<(SearchMatch, u64)>,
    count: u64,
}
impl Summary {
    fn record(
        &mut self,
        found: SearchMatch,
        direction: SearchDirection,
        anchor: Option<SearchPoint>,
    ) {
        self.count = self.count.saturating_add(1);
        self.first.get_or_insert(found);
        self.last = Some(found);
        let eligible = anchor.is_none_or(|a| match direction {
            SearchDirection::Next => found.start > a,
            SearchDirection::Previous => found.start < a,
            SearchDirection::Stay => found.start >= a,
        });
        if eligible && (self.best.is_none() || direction == SearchDirection::Previous) {
            self.best = Some((found, self.count));
        }
    }
    fn append(&mut self, batch: Self, direction: SearchDirection) {
        if let Some((found, ordinal)) = batch.best
            && (self.best.is_none() || direction == SearchDirection::Previous)
        {
            self.best = Some((found, self.count.saturating_add(ordinal)));
        }
        if self.first.is_none() {
            self.first = batch.first;
        }
        if batch.last.is_some() {
            self.last = batch.last;
        }
        self.count = self.count.saturating_add(batch.count);
    }
    fn finish(&self, direction: SearchDirection) -> SearchResult {
        let chosen = self.best.or_else(|| match direction {
            SearchDirection::Previous => self.last.map(|m| (m, self.count)),
            _ => self.first.map(|m| (m, 1)),
        });
        SearchResult {
            active: chosen.map(|(m, _)| m),
            ordinal: chosen.map_or(0, |(_, n)| n),
            count: self.count,
        }
    }
}
#[derive(Debug, Default, PartialEq, Eq)]
struct LogicalLine {
    text: String,
    points: Vec<(usize, SearchPoint)>,
}
#[derive(Debug)]
pub struct RegexWork {
    regex: Arc<regex::Regex>,
    lines: Vec<LogicalLine>,
    generation: u64,
    direction: SearchDirection,
    anchor: Option<SearchPoint>,
    cancelled: Arc<AtomicBool>,
}
impl PartialEq for RegexWork {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.regex, &other.regex)
            && self.lines == other.lines
            && self.generation == other.generation
            && self.direction == other.direction
            && self.anchor == other.anchor
    }
}
impl Eq for RegexWork {}
#[derive(Debug)]
pub struct SearchBatch {
    generation: u64,
    summary: Summary,
}
impl RegexWork {
    fn check_budget(&self, started: Instant) -> Result<(), String> {
        if self.cancelled.load(Ordering::Relaxed) {
            return Err("Search was cancelled.".into());
        }
        if started.elapsed() > MAX_REGEX_WORK_TIME {
            return Err(
                "Regular expression exceeded the search work budget. Simplify the pattern.".into(),
            );
        }
        Ok(())
    }
    /// No terminal state is accessed here. No list of matches is retained.
    pub fn run(self) -> Result<SearchBatch, String> {
        let started = Instant::now();
        let mut summary = Summary::default();
        for line in &self.lines {
            self.check_budget(started)?;
            for found in self.regex.find_iter(&line.text) {
                self.check_budget(started)?;
                if found.is_empty() {
                    continue;
                }
                let start = line
                    .points
                    .partition_point(|(byte, _)| *byte < found.start());
                let end = line.points.partition_point(|(byte, _)| *byte < found.end());
                if let (Some((_, start)), Some((_, end))) = (
                    line.points.get(start),
                    end.checked_sub(1).and_then(|end| line.points.get(end)),
                ) {
                    summary.record(
                        SearchMatch {
                            start: *start,
                            end: *end,
                        },
                        self.direction,
                        self.anchor,
                    );
                }
            }
            self.check_budget(started)?;
        }
        Ok(SearchBatch {
            generation: self.generation,
            summary,
        })
    }
}
struct Cursor {
    line: i32,
    end_line: i32,
    column: usize,
    part: usize,
    trim: Option<usize>,
    end_column: usize,
    wrapped: bool,
}
enum Tick {
    Inspection,
    Scalar(char, SearchPoint),
    End,
}
impl Cursor {
    fn new(e: &Emulator) -> Self {
        Self {
            line: e.search_grid().topmost_line().0,
            end_line: e.search_grid().bottommost_line().0,
            column: 0,
            part: 0,
            trim: None,
            end_column: 0,
            wrapped: false,
        }
    }
    fn tick(&mut self, e: &Emulator) -> Tick {
        if self.line > self.end_line {
            return Tick::End;
        }
        let grid = e.search_grid();
        if self.trim.is_none() {
            self.wrapped = grid[Line(self.line)][Column(grid.columns() - 1)]
                .flags
                .contains(Flags::WRAPLINE);
            self.trim = Some(grid.columns());
        }
        if self.column == 0 && self.part == 0 {
            let trim = self.trim.unwrap();
            if !self.wrapped && trim > 0 {
                let cell = &grid[Line(self.line)][Column(trim - 1)];
                if cell.c == ' ' && cell.zerowidth().is_none_or(|s| s.is_empty()) {
                    self.trim = Some(trim - 1);
                    return Tick::Inspection;
                }
            }
            self.end_column = trim;
        }
        if self.column >= self.end_column {
            let point = SearchPoint {
                line: self.line,
                column: self.end_column.min(grid.columns() - 1) as u16,
                part: usize::MAX,
            };
            let wrapped = self.wrapped;
            self.line += 1;
            self.column = 0;
            self.part = 0;
            self.trim = None;
            return if wrapped {
                Tick::Inspection
            } else {
                Tick::Scalar('\n', point)
            };
        }
        let cell = &grid[Line(self.line)][Column(self.column)];
        if cell
            .flags
            .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
        {
            self.column += 1;
            self.part = 0;
            return Tick::Inspection;
        }
        let scalar = if self.part == 0 {
            Some(cell.c)
        } else {
            cell.zerowidth().and_then(|s| s.get(self.part - 1)).copied()
        };
        if let Some(scalar) = scalar {
            let point = SearchPoint {
                line: self.line,
                column: self.column as u16,
                part: self.part,
            };
            self.part += 1;
            Tick::Scalar(scalar, point)
        } else {
            self.column += 1;
            self.part = 0;
            Tick::Inspection
        }
    }
}
#[derive(Default)]
struct Fold {
    cache: HashMap<char, char>,
}
impl Fold {
    fn scalar(&mut self, c: char) -> char {
        if let Some(folded) = self.cache.get(&c) {
            return *folded;
        }
        let mut class =
            regex_syntax::hir::ClassUnicode::new([regex_syntax::hir::ClassUnicodeRange::new(c, c)]);
        class
            .try_case_fold_simple()
            .expect("Unicode case tables are enabled");
        let folded = class.iter().next().map_or(c, |range| range.start());
        if self.cache.len() >= 4096 {
            self.cache.clear();
        }
        self.cache.insert(c, folded);
        folded
    }
}
struct Literal {
    query: Vec<char>,
    prefix: Vec<usize>,
    ring: VecDeque<(SearchPoint, bool, bool)>,
    matched: usize,
    fold: Fold,
    previous_word: bool,
    pending: Option<(SearchMatch, bool)>,
}
struct RegexState {
    regex: Arc<regex::Regex>,
    line: LogicalLine,
    lines: Vec<LogicalLine>,
    bytes: usize,
    waiting: bool,
}
enum Matcher {
    Literal(Literal),
    Regex(RegexState),
}
/// Retains only the literal query or a bounded batch of complete regex lines.
pub struct SearchScan {
    generation: u64,
    options: SearchOptions,
    cursor: Cursor,
    matcher: Matcher,
    anchor: Option<SearchPoint>,
    direction: SearchDirection,
    summary: Summary,
    complete: Option<SearchResult>,
    error: Option<String>,
    cancelled: Arc<AtomicBool>,
}
impl Drop for SearchScan {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
}
impl SearchScan {
    pub fn provisional(&self) -> Option<SearchMatch> {
        if self.direction == SearchDirection::Previous {
            None
        } else {
            self.summary.best.map(|(found, _)| found)
        }
    }
    pub(crate) fn new(
        e: &Emulator,
        query: &str,
        options: SearchOptions,
        direction: SearchDirection,
        anchor: Option<SearchPoint>,
    ) -> Result<Self, String> {
        let mut chars: Vec<_> = query.chars().take(MAX_SEARCH_QUERY + 1).collect();
        if chars.len() > MAX_SEARCH_QUERY {
            return Err(format!(
                "Search is limited to {MAX_SEARCH_QUERY} characters."
            ));
        }
        if chars.is_empty() {
            return Err("Enter text to search.".into());
        }
        let matcher = if options.regex {
            let pattern = if options.whole_word {
                format!(r"\b(?:{query})\b")
            } else {
                query.to_owned()
            };
            let regex = regex::RegexBuilder::new(&pattern)
                .case_insensitive(!options.case_sensitive)
                .size_limit(MAX_REGEX_COMPILED_BYTES)
                .dfa_size_limit(MAX_REGEX_COMPILED_BYTES)
                .build()
                .map_err(|error| format!("Invalid or oversized regular expression: {error}"))?;
            Matcher::Regex(RegexState {
                regex: Arc::new(regex),
                line: LogicalLine::default(),
                lines: Vec::new(),
                bytes: 0,
                waiting: false,
            })
        } else {
            let mut fold = Fold::default();
            if !options.case_sensitive {
                for c in &mut chars {
                    *c = fold.scalar(*c);
                }
            }
            let mut prefix = vec![0; chars.len()];
            for i in 1..chars.len() {
                let mut j = prefix[i - 1];
                while j > 0 && chars[i] != chars[j] {
                    j = prefix[j - 1];
                }
                if chars[i] == chars[j] {
                    j += 1;
                }
                prefix[i] = j;
            }
            Matcher::Literal(Literal {
                query: chars,
                prefix,
                ring: VecDeque::new(),
                matched: 0,
                fold,
                previous_word: false,
                pending: None,
            })
        };
        Ok(Self {
            generation: e.generation(),
            options,
            cursor: Cursor::new(e),
            matcher,
            anchor,
            direction,
            summary: Summary::default(),
            complete: None,
            error: None,
            cancelled: Arc::new(AtomicBool::new(false)),
        })
    }
    fn work(&mut self) -> Option<SearchProgress> {
        let Matcher::Regex(state) = &mut self.matcher else {
            return None;
        };
        if state.lines.is_empty() || state.waiting {
            return None;
        }
        state.waiting = true;
        state.bytes = 0;
        Some(SearchProgress::Work(RegexWork {
            regex: state.regex.clone(),
            lines: std::mem::take(&mut state.lines),
            generation: self.generation,
            direction: self.direction,
            anchor: self.anchor,
            cancelled: self.cancelled.clone(),
        }))
    }
    pub fn accept_work(
        &mut self,
        e: &Emulator,
        result: Result<SearchBatch, String>,
    ) -> SearchProgress {
        if e.generation() != self.generation {
            return SearchProgress::Invalidated;
        }
        let Matcher::Regex(state) = &mut self.matcher else {
            return SearchProgress::Failed("Unexpected regular expression work.".into());
        };
        state.waiting = false;
        match result {
            Ok(batch) if batch.generation == self.generation => {
                self.summary.append(batch.summary, self.direction)
            }
            Ok(_) => return SearchProgress::Invalidated,
            Err(error) => {
                self.error = Some(error.clone());
                return SearchProgress::Failed(error);
            }
        }
        SearchProgress::Searching
    }
    /// Visits at most budget grid inspections or Unicode scalars on this thread.
    pub fn step(&mut self, e: &Emulator, budget: usize) -> SearchProgress {
        if e.generation() != self.generation {
            return SearchProgress::Invalidated;
        }
        if let Some(error) = &self.error {
            return SearchProgress::Failed(error.clone());
        }
        if let Some(result) = self.complete {
            return SearchProgress::Complete(result);
        }
        if matches!(&self.matcher,Matcher::Regex(state) if state.waiting) {
            return SearchProgress::Searching;
        }
        for _ in 0..budget {
            if matches!(&self.matcher,Matcher::Regex(state) if !state.lines.is_empty() && state.bytes+state.line.text.len()>=MAX_REGEX_BATCH_BYTES)
                && let Some(work) = self.work()
            {
                return work;
            }
            match self.cursor.tick(e) {
                Tick::Inspection => {}
                Tick::End => {
                    if let Matcher::Regex(state) = &mut self.matcher
                        && !state.line.text.is_empty()
                    {
                        state.lines.push(std::mem::take(&mut state.line));
                    }
                    if let Some(work) = self.work() {
                        return work;
                    }
                    if let Matcher::Literal(state) = &mut self.matcher
                        && let Some((found, true)) = state.pending.take()
                    {
                        self.summary.record(found, self.direction, self.anchor);
                    }
                    let result = self.summary.finish(self.direction);
                    self.complete = Some(result);
                    return SearchProgress::Complete(result);
                }
                Tick::Scalar(scalar, point) => match &mut self.matcher {
                    Matcher::Literal(state) => {
                        let word = regex_syntax::is_word_character(scalar);
                        if let Some((found, end_word)) = state.pending.take()
                            && end_word != word
                        {
                            self.summary.record(found, self.direction, self.anchor);
                        }
                        state.ring.push_back((point, state.previous_word, word));
                        state.previous_word = word;
                        if state.ring.len() > state.query.len() {
                            state.ring.pop_front();
                        }
                        let scalar = if self.options.case_sensitive {
                            scalar
                        } else {
                            state.fold.scalar(scalar)
                        };
                        while state.matched > 0 && scalar != state.query[state.matched] {
                            state.matched = state.prefix[state.matched - 1];
                        }
                        if scalar == state.query[state.matched] {
                            state.matched += 1;
                        }
                        if state.matched == state.query.len() {
                            let (start, previous_word, start_word) = *state.ring.front().unwrap();
                            let found = SearchMatch { start, end: point };
                            if !self.options.whole_word {
                                self.summary.record(found, self.direction, self.anchor);
                            } else if previous_word != start_word {
                                state.pending = Some((found, word));
                            }
                            state.matched = state.prefix[state.matched - 1];
                        }
                    }
                    Matcher::Regex(state) => {
                        if scalar == '\n' && point.part == usize::MAX {
                            state.bytes += state.line.text.len();
                            state.lines.push(std::mem::take(&mut state.line));
                        } else {
                            if state.line.text.len() + scalar.len_utf8() > MAX_REGEX_LINE_BYTES {
                                let error = format!(
                                    "Regular expression search is limited to {MAX_REGEX_LINE_BYTES} bytes per logical line. Use literal mode for longer lines."
                                );
                                self.error = Some(error.clone());
                                return SearchProgress::Failed(error);
                            }
                            state.line.points.push((state.line.text.len(), point));
                            state.line.text.push(scalar);
                        }
                    }
                },
            }
        }
        self.work().unwrap_or(SearchProgress::Searching)
    }
}

#[cfg(test)]
mod tests;
