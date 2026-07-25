mod model;

fn main() {
    let request = serde_json::from_reader(std::io::stdin()).expect("valid contract request");
    serde_json::to_writer(std::io::stdout(), &model::run(request))
        .expect("write contract response");
}
