use nve_abi::ReprEnum;

#[repr(u8)]
#[derive(ReprEnum)]
enum TupleVar {
    A(u8) = 0,
}

#[repr(u8)]
#[derive(ReprEnum)]
enum StructVar {
    A { x: u8 } = 0,
}

#[repr(u8)]
#[derive(ReprEnum)]
enum EmptyEnum {}

#[repr(u8)]
#[derive(ReprEnum)]
enum GenericEnum<T> {
    A = 0,
}

#[repr(u8)]
#[derive(ReprEnum)]
enum WhereEnum
where
    u8: Copy,
{
    A = 0,
}

fn main() {}
