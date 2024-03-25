mod types;
mod bandcamp;

fn main() {
    let html = include_str!("../test/content.html");
    let info = bandcamp::BlogInfo::from_html(html);
    println!("{:#?}", info);
}
