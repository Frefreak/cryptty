// Observe string boundaries while vte handles terminal text and controls.
#[derive(Default)]
pub struct Stream {
    state: State,
    command: Vec<u8>,
    overflow: bool,
}

#[derive(Default)]
enum State {
    #[default]
    Ground,
    Escape,
    String {
        osc: bool,
    },
    Apc,
    ApcEscape,
}

pub enum Event<'a> {
    Text(&'a [u8]),
    Graphics(&'a [u8]),
}

impl Stream {
    pub fn feed(&mut self, mut bytes: &[u8], mut emit: impl FnMut(Event<'_>)) {
        while let Some((&byte, rest)) = bytes.split_first() {
            match self.state {
                State::Ground => {
                    let end = memchr::memchr(0x1b, bytes)
                        .map_or(bytes.len(), |i| i + 1);
                    emit(Event::Text(&bytes[..end]));
                    if bytes[end - 1] == 0x1b {
                        self.state = State::Escape;
                    }
                    bytes = &bytes[end..];
                    continue;
                }
                State::Escape => {
                    self.state = match byte {
                        b'_' => {
                            self.command.clear();
                            self.overflow = false;
                            State::Apc
                        }
                        b']' => State::String { osc: true },
                        b'P' | b'X' | b'^' => State::String { osc: false },
                        0x1b => State::Escape,
                        _ => State::Ground,
                    };
                    emit(Event::Text(&bytes[..1]));
                }
                State::String { osc } => {
                    if byte == 0x1b {
                        self.state = State::Escape;
                    } else if matches!(byte, 0x18 | 0x1a) || (osc && byte == 7)
                    {
                        self.state = State::Ground;
                    }
                    emit(Event::Text(&bytes[..1]));
                }
                State::Apc => match byte {
                    0x1b => self.state = State::ApcEscape,
                    0x18 | 0x1a => {
                        self.state = State::Ground;
                        emit(Event::Text(&bytes[..1]));
                    }
                    _ => {
                        if self.command.len() < 65536 {
                            self.command.push(byte);
                        } else {
                            self.overflow = true;
                        }
                    }
                },
                State::ApcEscape => {
                    emit(Event::Text(b"\x1b"));
                    if byte == b'\\' {
                        emit(Event::Text(b"\\"));
                        if !self.overflow && self.command.first() == Some(&b'G')
                        {
                            emit(Event::Graphics(&self.command[1..]));
                        }
                        self.state = State::Ground;
                    } else {
                        self.state = State::Escape;
                        continue;
                    }
                }
            }
            bytes = rest;
        }
    }
}
