#[global_allocator]
static ALLOC: wee_alloc::WeeAlloc = wee_alloc::WeeAlloc::INIT;

use core::mem;
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
extern "C" {
    type Api;

    #[wasm_bindgen(method, js_name = onLine)]
    fn on_line(api: &Api, addr: u32, size: u32, line: u32, file_index: u32);
    #[wasm_bindgen(method, js_name = onFunc)]
    fn on_func(api: &Api, addr: u32, size: u32, params: u32);
    #[wasm_bindgen(method, js_name = onFile)]
    fn on_file(api: &Api, index: u32);
    #[wasm_bindgen(method, js_name = onPublic)]
    fn on_public(api: &Api, addr: u32, params: u32);
    #[wasm_bindgen(method, js_name = onStrValue)]
    fn on_str_value(api: &Api, value: &[u8]);
}

#[allow(dead_code)]
#[derive(Copy, Clone)]
#[repr(u8)]
enum State {
    Start = 0,

    LineHexAddr = 1,
    LineHexSize = 2,
    LineDecLine = 3,
    LineDecFile = 4,
    LineEnd = 5,

    Skip = 6,

    FuncOrFile = 7,

    Func = 8,
    FuncHexAddr = 9,
    FuncHexSize = 10,
    FuncHexParams = 11,
    FuncStrName = 12,
    FuncEnd = 13,

    File = 14,
    FileDecIndex = 15,
    FileStrName = 16,
    FileEnd = 17,

    Public = 18,
    PublicHexAddr = 19,
    PublicHexParams = 20,
    PublicStrName = 21,
    PublicEnd = 22,
}

impl State {
    fn next(self) -> Self {
        unsafe { mem::transmute(self as u8 + 1) }
    }
}

const DEC_TABLE: [u8; 256] = {
    let mut output = [0xffu8; 256];

    let mut i = b'0';
    while i <= b'9' {
        output[i as usize] = i - b'0';
        i += 1;
    }

    output
};

const fn dec_value(ch: u8) -> Option<u8> {
    let val = DEC_TABLE[ch as usize];
    if val == 0xff {
        None
    } else {
        Some(val)
    }
}

const HEX_TABLE: [u8; 256] = {
    let mut output = DEC_TABLE;

    let mut i = b'a';
    while i <= b'f' {
        output[i as usize] = i - b'a' + 0x0a;
        i += 1;
    }

    output
};

const fn hex_value(ch: u8) -> Option<u8> {
    let val = HEX_TABLE[ch as usize];
    if val == 0xff {
        None
    } else {
        Some(val)
    }
}

#[wasm_bindgen]
struct Parser {
    state: State,
    row: [u32; 4],
    row_pos: u8,
    api: Api,
}

#[wasm_bindgen]
impl Parser {
    #[wasm_bindgen(constructor)]
    pub fn new(api: Api) -> Self {
        Self {
            state: State::Start,
            row: [0; 4],
            row_pos: 0,
            api,
        }
    }

    #[wasm_bindgen]
    pub fn parse(&mut self, chunk: &[u8]) {
        let mut offset: usize = 0;
        while offset < chunk.len() {
            offset = match self.state {
                State::Start => self.parse_start(chunk, offset),
                State::FuncOrFile => self.parse_func_or_file(chunk, offset),

                State::LineHexAddr => self.parse_hex(chunk, offset),
                State::LineHexSize => self.parse_hex(chunk, offset),
                State::FuncHexAddr => self.parse_hex(chunk, offset),
                State::FuncHexSize => self.parse_hex(chunk, offset),
                State::FuncHexParams => self.parse_hex(chunk, offset),
                State::PublicHexAddr => self.parse_hex(chunk, offset),
                State::PublicHexParams => self.parse_hex(chunk, offset),

                State::LineDecLine => self.parse_dec(chunk, offset),
                State::LineDecFile => self.parse_dec(chunk, offset),
                State::FileDecIndex => self.parse_dec(chunk, offset),

                State::FuncStrName => self.parse_str(chunk, offset),
                State::FileStrName => self.parse_str(chunk, offset),
                State::PublicStrName => self.parse_str(chunk, offset),

                State::Func => self.skip_until_digit(chunk, offset),
                State::File => self.skip_until_digit(chunk, offset),
                State::Public => self.skip_until_digit(chunk, offset),

                State::LineEnd => {
                    self.on_line_end();
                    offset
                }
                State::FuncEnd => {
                    self.on_func_end();
                    offset
                }
                State::FileEnd => {
                    self.on_file_end();
                    offset
                }
                State::PublicEnd => {
                    self.on_public_end();
                    offset
                }

                State::Skip => self.skip_past_newline(chunk, offset),
            }
        }
    }

    #[wasm_bindgen]
    pub fn finish(&mut self) {
        match self.state {
            State::LineEnd => self.on_line_end(),
            State::FuncEnd => self.on_func_end(),
            State::FileEnd => self.on_file_end(),
            State::PublicEnd => self.on_public_end(),
            _ => (),
        }
    }

    fn parse_start(&mut self, chunk: &[u8], offset: usize) -> usize {
        let ch = chunk[offset];
        if hex_value(ch).is_some() {
            self.state = State::LineHexAddr;

            // First character is significant
            return offset;
        }

        self.state = match ch {
            b'F' => State::FuncOrFile,
            b'P' => State::Public,

            // Likely STACK
            _ => State::Skip,
        };

        offset + 1
    }

    fn parse_func_or_file(&mut self, chunk: &[u8], offset: usize) -> usize {
        self.state = match chunk[offset] {
            b'U' => State::Func,
            b'I' => State::File,
            _ => State::Skip,
        };
        offset + 1
    }

    fn parse_hex(&mut self, chunk: &[u8], offset: usize) -> usize {
        let mut int_value = self.row[self.row_pos as usize];
        for i in offset..chunk.len() {
            let d = match hex_value(chunk[i]) {
                Some(d) => d,
                None => {
                    self.row_pos += 1;
                    self.state = self.state.next();
                    return i + 1;
                }
            };

            int_value = (int_value << 4) | u32::from(d);
        }
        self.row[self.row_pos as usize] = int_value;
        chunk.len()
    }

    fn parse_dec(&mut self, chunk: &[u8], offset: usize) -> usize {
        let mut int_value = self.row[self.row_pos as usize];
        for i in offset..chunk.len() {
            let d = match dec_value(chunk[i]) {
                Some(d) => d,
                None => {
                    self.row[self.row_pos as usize] = int_value;
                    self.row_pos += 1;
                    self.state = self.state.next();
                    return i + 1;
                }
            };

            int_value = (int_value * 10) + u32::from(d);
        }
        self.row[self.row_pos as usize] = int_value;
        chunk.len()
    }

    fn parse_str(&mut self, chunk: &[u8], offset: usize) -> usize {
        for i in offset..chunk.len() {
            if chunk[i] == b'\n' {
                self.state = self.state.next();
                self.api.on_str_value(&chunk[offset..i]);
                return i + 1;
            }
        }
        self.api.on_str_value(&chunk[offset..chunk.len()]);
        chunk.len()
    }

    fn skip_until_digit(&mut self, chunk: &[u8], offset: usize) -> usize {
        for i in offset..chunk.len() {
            if hex_value(chunk[i]).is_some() {
                self.state = self.state.next();
                return i;
            }
        }
        chunk.len()
    }

    fn skip_past_newline(&mut self, chunk: &[u8], offset: usize) -> usize {
        for i in offset..chunk.len() {
            if chunk[i] == b'\n' {
                self.state = State::Start;
                return i + 1;
            }
        }
        chunk.len()
    }

    fn on_line_end(&mut self) {
        self.on_end();
        self.api
            .on_line(self.row[0], self.row[1], self.row[2], self.row[3]);
    }

    fn on_func_end(&mut self) {
        self.on_end();
        self.api.on_func(self.row[0], self.row[1], self.row[2]);
    }

    fn on_file_end(&mut self) {
        self.on_end();
        self.api.on_file(self.row[0]);
    }

    fn on_public_end(&mut self) {
        self.on_end();
        self.api.on_public(self.row[0], self.row[1]);
    }

    fn on_end(&mut self) {
        self.row_pos = 0;
        self.row = [0; 4];
        self.state = State::Start;
    }
}
