use vergen::{vergen, Config};

fn main() {
    vergen(Config::default()).expect("Unable to generate build information");
}
