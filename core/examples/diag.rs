//! Diagnose one file of a profile and print the result as JSON.
//! cargo run --release --example diag -- <game dir> <logs/latest.log | crash-reports/x.txt>
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let t = std::time::Instant::now();
    let d = obelisk_core::crash::diagnose(std::path::Path::new(&a[1]), &a[2]).unwrap();
    eprintln!("took {:.2}s", t.elapsed().as_secs_f32());
    let mut d = d;
    d.excerpt.truncate(12);
    println!("{}", serde_json::to_string(&d).unwrap());
}
