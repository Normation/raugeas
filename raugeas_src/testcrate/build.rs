extern crate raugeas_src;

fn main() {
    let artifacts = raugeas_src::build().unwrap();
    artifacts.print_cargo_metadata();
}
