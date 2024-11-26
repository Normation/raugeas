extern crate augeas_src;

fn main() {
    let artifacts = augeas_src::build().unwrap();
    artifacts.print_cargo_metadata();
}
