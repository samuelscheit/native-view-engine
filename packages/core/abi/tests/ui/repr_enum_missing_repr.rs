use nve_abi::ReprEnum;

#[derive(ReprEnum)]
enum NoRepr {
    A = 0,
}

#[repr(C)]
#[derive(ReprEnum)]
enum CRepr {
    A = 0,
}

#[repr(usize)]
#[derive(ReprEnum)]
enum UsizeRepr {
    A = 0,
}

fn main() {}
