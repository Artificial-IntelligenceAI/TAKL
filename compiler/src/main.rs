fn main() {
    let path = std::env::args().nth(1).unwrap_or_default();
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    match takl::check_program(&path, &text) {
        Ok(_) => println!("(checked)"),
        Err(shown) => print!("{}", shown),
    }
}
