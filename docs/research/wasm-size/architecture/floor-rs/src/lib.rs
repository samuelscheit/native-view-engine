//! no_std counter that mirrors NVE's architecture at its theoretical minimum:
//! - static UI is a compile-time HTML *template* (data segment), instantiated once by JS
//!   (`<template>` + `cloneNode`), never rebuilt node-by-node in wasm;
//! - dynamic work is encoded into ONE flat binary command buffer per frame (NVE `CommandBuffer`
//!   idea) and handed to a ~20-line JS interpreter with a single import call (`f(ptr,len)`);
//! - events arrive as plain i32 params (node slot, event kind): no serialization, no closures.
#![no_std]

use core::panic::PanicInfo;

#[panic_handler]
fn panic(_: &PanicInfo) -> ! {
    core::arch::wasm32::unreachable()
}

#[link(wasm_import_module = "env")]
unsafe extern "C" {
    /// Apply one frame: JS decodes the command buffer in linear memory.
    fn f(ptr: *const u8, len: usize);
}

const OP_TEMPLATE: u8 = 1; // op, tpl u8, html_len u16, html
const OP_MOUNT: u8 = 2; // op, tpl u8, base slot u16
const OP_TEXT: u8 = 3; // op, slot u16, len u16, utf8

const TPL: &[u8] = concat!(
    "<style>i{display:flex;flex-direction:column;box-sizing:border-box}b{font:16px sans-serif}</style>",
    "<i style=\"width:100%;height:100%;padding:24px;gap:16px;background:#0f172a\">",
    "<i style=\"padding:20px;gap:8px;background:#1e293b;border-radius:16px\">",
    "<b style=\"color:#38bdf8;font-size:14px\">NATIVE VIEW ENGINE</b>",
    "<b style=\"color:#fff;font-size:28px\">Count: <u data-n=0>0</u></b></i>",
    "<i style=\"flex-direction:row;gap:12px\">",
    "<i data-n=1 style=\"width:160px;height:48px;justify-content:center;align-items:center;background:#2563eb;border-radius:12px\"><b style=\"color:#fff\">Increment +</b></i>",
    "<i data-n=2 style=\"width:120px;height:48px;justify-content:center;align-items:center;background:#334155;border-radius:12px\"><b style=\"color:#e2e8f0\">Reset</b></i>",
    "</i></i>"
)
.as_bytes();

struct Frame {
    buf: [u8; 2048],
    len: usize,
}

impl Frame {
    fn push(&mut self, b: &[u8]) {
        self.buf[self.len..self.len + b.len()].copy_from_slice(b);
        self.len += b.len();
    }
    fn u16(&mut self, v: u16) {
        self.push(&v.to_le_bytes());
    }
    fn flush(&mut self) {
        unsafe { f(self.buf.as_ptr(), self.len) };
        self.len = 0;
    }
}

static mut FRAME: Frame = Frame { buf: [0; 2048], len: 0 };
static mut COUNT: i32 = 0;

#[allow(static_mut_refs)]
fn frame() -> &'static mut Frame {
    unsafe { &mut FRAME }
}

fn set_count(v: i32) {
    unsafe { COUNT = v };
    // i32 -> decimal (what `format!("{count}")` would pull core::fmt for).
    let mut tmp = [0u8; 11];
    let mut p = tmp.len();
    let mut n = v.unsigned_abs();
    loop {
        p -= 1;
        tmp[p] = b'0' + (n % 10) as u8;
        n /= 10;
        if n == 0 {
            break;
        }
    }
    if v < 0 {
        p -= 1;
        tmp[p] = b'-';
    }
    let fr = frame();
    fr.push(&[OP_TEXT]);
    fr.u16(0);
    fr.u16((tmp.len() - p) as u16);
    fr.push(&tmp[p..]);
    fr.flush();
}

#[unsafe(no_mangle)]
pub extern "C" fn start() {
    let fr = frame();
    fr.push(&[OP_TEMPLATE, 0]);
    fr.u16(TPL.len() as u16);
    fr.push(TPL);
    fr.push(&[OP_MOUNT, 0]);
    fr.u16(0);
    fr.flush();
}

/// Event entry: `slot` is the `data-n` of the closest listener node, `kind` the event type.
#[unsafe(no_mangle)]
pub extern "C" fn ev(slot: u32, _kind: u32) {
    match slot {
        1 => set_count(unsafe { COUNT } + 1),
        2 => set_count(0),
        _ => {}
    }
}
