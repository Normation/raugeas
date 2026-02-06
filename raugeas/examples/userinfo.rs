use raugeas::{Augeas, Flags};
use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    let aug = Augeas::init(Some("tests/test_root"), "", Flags::NONE)?;
    let username = "root";

    println!("Infos about '{}':", username);

    let info_nodes = aug.matches("etc/passwd/root/*")?;

    for node in info_nodes.iter() {
        let label = aug.label(node)?;
        let value = aug.get(node)?;

        if let (Some(label), Some(value)) = (label, value) {
            println!("{} = {}", label, value)
        }
    }
    Ok(())
}
