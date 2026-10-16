pub const DISAMBIGUATE: u8 = 1;
pub const REPORT_EVENTS: u8 = 2;
pub const REPORT_ALTERNATES: u8 = 4;
pub const REPORT_ALL: u8 = 8;
pub const REPORT_TEXT: u8 = 16;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Flags(u8);

impl Flags {
    pub fn new(bits: usize) -> Self {
        Self((bits & 31) as u8)
    }

    pub fn bits(self) -> u8 {
        self.0
    }

    pub fn contains(self, flag: u8) -> bool {
        self.0 & flag != 0
    }
}

#[derive(Default)]
pub struct ModeStack {
    flags: Flags,
    saved: Vec<Flags>,
}

impl ModeStack {
    pub fn flags(&self) -> Flags {
        self.flags
    }

    pub fn set(&mut self, flags: usize, mode: usize) {
        self.flags = match mode {
            1 => Flags::new(flags),
            2 => Flags::new(usize::from(self.flags.0) | flags),
            3 => Flags::new(usize::from(self.flags.0) & !flags),
            _ => return,
        };
    }

    pub fn push(&mut self, flags: usize) {
        if self.saved.len() == 32 {
            self.saved.remove(0);
        }
        self.saved.push(self.flags);
        self.flags = Flags::new(flags);
    }

    pub fn pop(&mut self, count: usize) {
        if count == 0 {
            return;
        }
        if count > self.saved.len() {
            self.saved.clear();
            self.flags = Flags::default();
        } else {
            let index = self.saved.len() - count;
            self.flags = self.saved[index];
            self.saved.truncate(index);
        }
    }
}
