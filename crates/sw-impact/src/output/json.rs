use sw_impact_scanner::report::Report;

pub fn output_json(report: &Report) {
    println!("{}", serde_json::to_string(report).unwrap());
}
