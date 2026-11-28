use nve_store::WordEnum;

#[repr(u8)]
#[derive(Clone, Copy, WordEnum)]
enum BadWordEnumUnset {
    Unset = 0,
    On = 1,
}

fn main() {}
