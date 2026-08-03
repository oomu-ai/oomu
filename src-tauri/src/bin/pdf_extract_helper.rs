#[path = "../pdf_helper.rs"]
mod pdf_helper;
#[path = "../pdf_protocol.rs"]
mod pdf_protocol;

fn main() {
    std::process::exit(pdf_helper::run());
}
