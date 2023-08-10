use stateright::{Checker, Model};

struct Adder;

impl Model for Adder {
    type State = usize;

    type Action = usize;

    fn init_states(&self) -> Vec<Self::State> {
        vec![0]
    }

    fn actions(&self, state: &Self::State, actions: &mut Vec<Self::Action>) {
        actions.push(1);
        actions.push(2);
        actions.push(3);
        actions.push(4);
        actions.push(5);
        let thread_name = std::thread::current().name().unwrap().to_owned();
        // dbg!(&thread_name);
        if thread_name.contains("2") {
            // if *state == 5000 {
            panic!()
        }
    }

    fn next_state(&self, last_state: &Self::State, action: Self::Action) -> Option<Self::State> {
        Some(last_state + action)
    }

    fn properties(&self) -> Vec<stateright::Property<Self>> {
        vec![stateright::Property::always("true", |_, _| true)]
    }
}

fn main() {
    env_logger::init_from_env(env_logger::Env::default().default_filter_or("info")); // `RUST_LOG=${LEVEL}` env variable to override
    Adder.checker().threads(3).spawn_dfs().join();
}
