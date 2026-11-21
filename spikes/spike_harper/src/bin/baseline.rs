//! The same build settings with nothing in it, to subtract from the spike's size.
fn main() {
    println!("{}", std::env::args().count());
}
