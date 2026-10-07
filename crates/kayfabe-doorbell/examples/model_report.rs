fn main() {
    match kayfabe_doorbell::model::check(kayfabe_doorbell::model::ClaimShape::SingleCas) {
        Err(w) => println!("COUNTEREXAMPLE:\n{w}"),
        Ok(n) => println!("no counterexample, {n} states"),
    }
    match kayfabe_doorbell::model::check(kayfabe_doorbell::model::ClaimShape::Retrying) {
        Err(w) => println!("RETRYING FAILED:\n{w}"),
        Ok(n) => println!("retrying: CLEAR over {n} reachable states"),
    }
}
