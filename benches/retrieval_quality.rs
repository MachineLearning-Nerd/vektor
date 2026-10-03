use std::time::Instant;

fn main() {
    let started = Instant::now();
    let report = vektor::benchmark::run_checked_in_benchmark()
        .expect("checked-in tokio benchmark fixture should run");

    println!("{}", report.render_markdown());
    println!("elapsed_ms: {}", started.elapsed().as_millis());
}
