use crate::pages::{get_pages, Page, PageOutput};
use std::time::Instant;

pub struct AppState {
    pub current_page: usize,
    pub max_brightness_nits: f32,
    pub paper_white_nits: f32,
    pub show_ui: bool,
    pub auto_cycle: bool,
    pub cycle_interval: f32,
    pub last_cycle_time: Instant,
    pub start_time: Instant,
    pages: Vec<Box<dyn Page>>,
}

impl AppState {
    pub fn new() -> Self {
        let now = Instant::now();
        Self {
            current_page: 0,
            max_brightness_nits: 1000.0,
            paper_white_nits: 200.0,
            show_ui: false,
            auto_cycle: false,
            cycle_interval: 5.0,
            last_cycle_time: now,
            start_time: now,
            pages: get_pages(),
        }
    }

    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    pub fn next_page(&mut self) {
        self.current_page = (self.current_page + 1) % self.pages.len();
        self.last_cycle_time = Instant::now();
    }

    pub fn prev_page(&mut self) {
        if self.current_page == 0 {
            self.current_page = self.pages.len() - 1;
        } else {
            self.current_page -= 1;
        }
        self.last_cycle_time = Instant::now();
    }

    pub fn toggle_ui(&mut self) {
        self.show_ui = !self.show_ui;
    }

    pub fn current_page_name(&self) -> &'static str {
        self.pages[self.current_page].name()
    }

    pub fn render_current_page(&self, width: u32, height: u32) -> PageOutput {
        let elapsed_time = self.start_time.elapsed().as_secs_f32();
        self.pages[self.current_page].render(width, height, self.max_brightness_nits, elapsed_time)
    }

    pub fn update(&mut self) {
        if self.auto_cycle {
            let elapsed = self.last_cycle_time.elapsed().as_secs_f32();
            if elapsed >= self.cycle_interval {
                self.next_page();
            }
        }
    }
}
