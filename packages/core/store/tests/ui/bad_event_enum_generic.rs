#[derive(Debug, Clone, Copy, PartialEq, Eq, nve_abi::EventEnum)]
#[repr(u8)]
enum GenericEvent<T> {
    Click = 0,
}

fn main() {}
