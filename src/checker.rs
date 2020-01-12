//! A model checker for a state machine.
//!
//! A small example that solves a [sliding puzzle](https://en.wikipedia.org/wiki/Sliding_puzzle)
//! follows. The technique involves describing how to play and then claiming that the puzzle is
//! unsolvable, for which the model checker finds a counterexample sequence of steps that does in
//! fact solve the puzzle.
//!
//! ```rust
//! use stateright::*;
//! use stateright::checker::*;
//!
//! #[derive(Clone, Debug, Eq, PartialEq)]
//! enum Slide { Down, Up, Right, Left }
//!
//! let puzzle = QuickMachine {
//!     init_states: || vec![vec![1, 4, 2,
//!                               3, 5, 8,
//!                               6, 7, 0]],
//!     actions: |_, actions| {
//!         actions.append(&mut vec![
//!             Slide::Down, Slide::Up, Slide::Right, Slide::Left
//!         ]);
//!     },
//!     next_state: |last_state, action| {
//!         let empty = last_state.iter().position(|x| *x == 0).unwrap();
//!         let empty_y = empty / 3;
//!         let empty_x = empty % 3;
//!         let maybe_from = match action {
//!             Slide::Down  if empty_y > 0 => Some(empty - 3), // above
//!             Slide::Up    if empty_y < 2 => Some(empty + 3), // below
//!             Slide::Right if empty_x > 0 => Some(empty - 1), // left
//!             Slide::Left  if empty_x < 2 => Some(empty + 1), // right
//!             _ => None
//!         };
//!         maybe_from.map(|from| {
//!             let mut next_state = last_state.clone();
//!             next_state[empty] = last_state[from];
//!             next_state[from] = 0;
//!             next_state
//!         })
//!     }
//! };
//! let solved = vec![0, 1, 2,
//!                   3, 4, 5,
//!                   6, 7, 8];
//! let mut checker = Checker::new(&puzzle, |_, state| { state != &solved });
//! assert_eq!(checker.check(100), CheckResult::Fail { state: solved.clone() });
//! assert_eq!(checker.path_to(&solved), vec![
//!     (vec![1, 4, 2,
//!           3, 5, 8,
//!           6, 7, 0], Slide::Down),
//!     (vec![1, 4, 2,
//!           3, 5, 0,
//!           6, 7, 8], Slide::Right),
//!     (vec![1, 4, 2,
//!           3, 0, 5,
//!           6, 7, 8], Slide::Down),
//!     (vec![1, 0, 2,
//!           3, 4, 5,
//!           6, 7, 8], Slide::Right)]);
//! ```

use crate::*;
use fxhash::FxHashMap;
use std::collections::hash_map::Entry;
use std::collections::VecDeque;

/// Model checking can be time consuming, so the library checks up to a fixed number of states then
/// returns. This approach allows the library to avoid tying up a thread indefinitely while still
/// maintaining adequate performance. This type represents the result of one of those checking
/// passes.
#[derive(Debug, Eq, PartialEq)]
pub enum CheckResult<State> {
    /// Indicates that the checker still has pending states.
    Incomplete,
    /// Indicates that checking completed, and the invariant was not violated.
    Pass,
    /// Indicates that checking completed, and the invariant did not hold.
    Fail {
        /// A state that violates the invariant.
        state: State
    }
}

/// Generates every state reachable by a state machine, and verifies that an invariant holds.
pub struct Checker<'a, SM, I>
where
    SM: 'a + StateMachine,
    I: Fn(&SM, &SM::State) -> bool,
{
    workers: Vec<Worker<'a, SM, I>>,
}

impl<'a, SM, I> Checker<'a, SM, I>
where
    SM: 'a + StateMachine,
    SM::State: Hash,
    I: Fn(&SM, &SM::State) -> bool,
{
    /// Initializes a fresh checker for a state machine.
    pub fn new(sm: &SM, invariant: I) -> Checker<SM, I>
    {
        Checker { workers: vec![Worker::init(sm, invariant)] }
    }

    /// Visits up to a specified number of states checking the model's invariant. May return
    /// earlier when all states have been checked or a state is found in which the invariant fails
    /// to hold. If the checker is using multiple workers, then each will visit the specified
    /// number of states.
    pub fn check(&mut self, max_count: usize) -> CheckResult<SM::State>
    where I: Send, SM: Sync, SM::State: Send
    {
        crossbeam_utils::thread::scope(|scope| {
            // 1. Kick off every worker.
            let mut threads = Vec::new();
            for worker in self.workers.iter_mut() {
                threads.push(scope.spawn(move |_| worker.check(max_count)));
            }

            // 2. Join.
            let mut results = Vec::new();
            for thread in threads {
                results.push(thread.join().unwrap());
            }

            // 3. Consolidate results.
            let all_passed = results.iter().all(|r| {
                match r {
                    CheckResult::Pass => true,
                    _ => false,
                }
            });
            if all_passed { return CheckResult::Pass }
            for result in results {
                if let CheckResult::Fail { state } = result {
                    return CheckResult::Fail { state };
                }
            }
            CheckResult::Incomplete
        }).unwrap()
    }

    /// Identifies the action-state "behavior" path by which a generated state was reached.
    pub fn path_to(&self, state: &SM::State) -> Vec<(SM::State, SM::Action)> {
        // First build a stack of digests representing the path (with the init digest at top of
        // stack). Then unwind the stack of digests into a vector of states. The TLC model checker
        // uses a similar technique, which is documented in the paper "Model Checking TLA+
        // Specifications" by Yu, Manolios, and Lamport.

        let state_machine = self.workers.first().unwrap().state_machine;
        let sources = self.sources();

        // 1. Build a stack of digests.
        let mut digests = Vec::new();
        let mut next_digest = fingerprint(&state);
        while let Some(source) = sources.get(&next_digest) {
            match *source {
                Some(prev_digest) => {
                    digests.push(next_digest);
                    next_digest = prev_digest;
                },
                None => {
                    digests.push(next_digest);
                    break;
                },
            }
        }

        // 2. Begin unwinding by determining the init step.
        let init_states = state_machine.init_states();
        let mut last_state = init_states.into_iter().find(|s| fingerprint(&s) == digests.pop().unwrap()).unwrap();

        // 3. Then continue with the remaining steps.
        let mut output = Vec::new();
        while let Some(next_digest) = digests.pop() {
            let mut actions = Vec::new();
            state_machine.actions(
                &last_state,
                &mut actions);

            let (action, next_state) = actions.into_iter()
                .find_map(|action| {
                    state_machine.next_state(&last_state, &action)
                        .and_then(|next_state| {
                            if fingerprint(&next_state) == next_digest {
                                Some((action, next_state))
                            } else {
                                None
                            }
                        })
                })
                .expect("state matching recorded digest");
            output.push((last_state, action));

            last_state = next_state;
        }
        output
    }

    /// Blocks the thread until model checking is complete. Periodically emits a status while
    /// checking, tailoring the block size to the checking speed. Emits a report when complete.
    pub fn check_and_report(&mut self, w: &mut impl std::io::Write)
    where
        I: Copy + Send,
        SM: Sync,
        SM::State: Debug + Send,
        SM::Action: Debug,
    {
        use std::cmp::max;
        use std::time::Instant;

        let num_cpus = num_cpus::get();
        let method_start = Instant::now();
        let mut block_size = 32_768;
        loop {
            let block_start = Instant::now();
            match self.check(block_size) {
                CheckResult::Fail { state } => {
                    // First a quick summary.
                    let path = self.path_to(&state);
                    writeln!(w, "{} states pending after {} sec. Invariant violated by path of length {}.",
                             self.pending_count(),
                             method_start.elapsed().as_secs(),
                             path.len()).unwrap();

                    // Then show the path.
                    let state_machine = self.workers[0].state_machine;
                    for (state, action) in path {
                        writeln!(w, "ACTION: {:?}", action).unwrap();
                        if let Some(outcome) = state_machine.display_outcome(&state, &action) {
                            writeln!(w, "OUTCOME: {}", outcome).unwrap();
                        }
                    }
                    return;
                },
                CheckResult::Pass => {
                    println!("Passed after {} sec.",
                             method_start.elapsed().as_secs());
                    return;
                },
                CheckResult::Incomplete => {}
            }

            let block_elapsed = block_start.elapsed().as_secs();
            if block_elapsed > 0 {
                println!("{} states pending after {} sec. Continuing.",
                         self.pending_count(),
                         method_start.elapsed().as_secs());
            }

            // Shrink or grow block if necessary. Otherwise adjust workers based on block size.
            if block_elapsed < 2 { block_size = 3 * block_size / 2; }
            else if block_elapsed > 10 { block_size = max(1, block_size / 2); }
            else {
                let threshold = max(1, block_size / num_cpus / 2);
                let queues: Vec<_> = self.workers.iter()
                    .map(|w| w.pending.len()).collect();
                println!("  cores={} threshold={} queues={:?}",
                         num_cpus, threshold, queues);
                self.adjust_worker_count(num_cpus, threshold);
            }
        }
    }

    /// By default a checker has one worker. This method forks workers whose pending queue size
    /// exceeds a specified threshold (while staying below a target worker count).
    pub fn adjust_worker_count(&mut self, target: usize, min_pending: usize)
    where I: Copy
    {
        let mut added = Vec::new();
        loop {
            let existing_count = self.workers.iter()
                .filter(|w| !w.pending.is_empty()).count();
            for worker in &mut self.workers {
                if existing_count + added.len() >= target { break }
                if worker.pending.len() < min_pending { continue }
                added.push(worker.fork());
            }

            if added.is_empty() { return }
            self.workers.append(&mut added);
        }
    }

    /// Indicates how many states are pending. If extra workers were created, this number may
    /// include duplicates.
    pub fn pending_count(&self) -> usize {
        self.workers.iter().map(|w| w.pending.len()).sum()
    }

    /// Indicates state sources by digest.
    pub fn sources(&self) -> FxHashMap<u64, Option<u64>> {
        let max_capacity = self.workers.iter().map(|w| w.sources.capacity()).max().unwrap();
        let mut sources = FxHashMap::with_capacity_and_hasher(2 * max_capacity, Default::default());
        for worker in &self.workers { sources.extend(worker.sources.clone()); }
        sources
    }
}

struct Worker<'a, SM, I>
where
    SM: 'a + StateMachine,
    I: Fn(&SM, &SM::State) -> bool,
{
    // immutable cfg
    invariant: I,
    state_machine: &'a SM,

    // mutable checking state
    pending: VecDeque<SM::State>,
    sources: FxHashMap<u64, Option<u64>>,
}

impl<'a, SM, I> Worker<'a, SM, I>
where
    SM: 'a + StateMachine,
    SM::State: Hash,
    I: Fn(&SM, &SM::State) -> bool,
{
    fn init(state_machine: &'a SM, invariant: I) -> Worker<'a, SM, I> {
        const STARTING_CAPACITY: usize = 1_000_000;

        let mut pending = VecDeque::new();
        let mut sources = FxHashMap::with_capacity_and_hasher(STARTING_CAPACITY, Default::default());
        for init_state in state_machine.init_states() {
            let init_digest = fingerprint(&init_state);
            if let Entry::Vacant(init_source) = sources.entry(init_digest) {
                init_source.insert(None);
                pending.push_back(init_state);
            }
        }

        Worker {
            invariant,
            state_machine,

            pending,
            sources,
        }
    }

    fn fork(&mut self) -> Worker<'a, SM, I>
    where I: Copy
    {
        let len = self.pending.len() / 2;
        Worker {
            invariant: self.invariant,
            state_machine: self.state_machine,

            pending: self.pending.split_off(len),
            sources: self.sources.clone(),
        }
    }

    fn check(&mut self, max_count: usize) -> CheckResult<SM::State> {
        let mut remaining = max_count;
        let mut next_actions = Vec::new(); // reused between iterations for efficiency

        while let Some(state) = self.pending.pop_front() {
            let digest = fingerprint(&state);

            // collect the next actions, and record the corresponding states that have not been
            // seen before
            next_actions.clear();
            self.state_machine.actions(&state, &mut next_actions);
            for next_action in &next_actions {
                if let Some(next_state) = self.state_machine.next_state(&state, &next_action) {
                    let next_digest = fingerprint(&next_state);
                    if let Entry::Vacant(next_entry) = self.sources.entry(next_digest) {
                        next_entry.insert(Some(digest));
                        self.pending.push_back(next_state);
                    }
                }
            }

            // exit if invariant fails to hold or we've reached the max count
            let inv = &self.invariant;
            if !inv(&self.state_machine, &state) { return CheckResult::Fail { state }; }
            remaining -= 1;
            if remaining == 0 { return CheckResult::Incomplete }
        }

        CheckResult::Pass
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::test_util::linear_equation_solver::*;

    #[test]
    fn model_check_records_states() {
        use fxhash::FxHashSet;
        use std::iter::FromIterator;

        let h = |a: u8, b: u8| fingerprint(&(a, b));
        let mut checker = Checker::new(&LinearEquation { a: 2, b: 10, c: 14 }, invariant);
        checker.check(100);
        let state_space = FxHashSet::from_iter(checker.sources().keys().cloned());
        assert!(state_space.contains(&h(0, 0)));
        assert!(state_space.contains(&h(1, 0)));
        assert!(state_space.contains(&h(0, 1)));
        assert!(state_space.contains(&h(2, 0)));
        assert!(state_space.contains(&h(1, 1)));
        assert!(state_space.contains(&h(0, 2)));
        assert!(state_space.contains(&h(3, 0)));
        assert!(state_space.contains(&h(2, 1)));
        assert_eq!(state_space.len(), 13); // not all generated were checked
    }

    #[test]
    fn model_check_can_pass() {
        let mut checker = Checker::new(&LinearEquation { a: 2, b: 4, c: 7 }, invariant);
        assert_eq!(checker.check(100), CheckResult::Incomplete);
        assert_eq!(checker.sources().len(), 115); // not all generated were checked
        assert_eq!(checker.check(100_000), CheckResult::Pass);
        assert_eq!(checker.sources().len(), 256 * 256);
    }

    #[test]
    fn model_check_can_fail() {
        let mut checker = Checker::new(&LinearEquation { a: 2, b: 7, c: 111 }, invariant);
        assert_eq!(checker.check(100), CheckResult::Incomplete);
        assert_eq!(checker.sources().len(), 115); // not all generated were checked
        assert_eq!(
            checker.check(100_000),
            CheckResult::Fail { state: (3, 15) });
        assert_eq!(checker.sources().len(), 207); // only 187 were checked
    }

    #[test]
    fn model_check_can_resume_after_failing() {
        let mut checker = Checker::new(&LinearEquation { a: 0, b: 0, c: 0 }, invariant);
        // init case
        assert_eq!(checker.check(100), CheckResult::Fail { state: (0, 0) });
        // distance==1 cases
        assert_eq!(checker.check(100), CheckResult::Fail { state: (1, 0) });
        assert_eq!(checker.check(100), CheckResult::Fail { state: (0, 1) });
        // subset of distance==2 cases
        assert_eq!(checker.check(100), CheckResult::Fail { state: (2, 0) });
        assert_eq!(checker.check(100), CheckResult::Fail { state: (1, 1) });
        assert_eq!(checker.check(100), CheckResult::Fail { state: (0, 2) });
    }

    #[test]
    fn model_check_can_indicate_path() {
        let mut checker = Checker::new(&LinearEquation { a: 2, b: 10, c: 14 }, invariant);
        match checker.check(100_000) {
            CheckResult::Fail { state } => {
                assert_eq!(
                    checker.path_to(&state),
                    vec![
                        ((0, 0), Guess::IncreaseX),
                        ((1, 0), Guess::IncreaseX),
                        ((2, 0), Guess::IncreaseY),
                    ]);
            },
            _ => panic!("expected solution")
        }
    }

    #[test]
    fn report_includes_path() {
        let mut checker = Checker::new(&LinearEquation { a: 2, b: 10, c: 14 }, invariant);
        let mut written: Vec<u8> = Vec::new();
        checker.check_and_report(&mut written);
        let output = String::from_utf8(written).unwrap();
        assert_eq!(
            output,
            "5 states pending after 0 sec. Invariant violated by path of length 3.\n\
             ACTION: IncreaseX\n\
             OUTCOME: (1, 0)\n\
             ACTION: IncreaseX\n\
             OUTCOME: (2, 0)\n\
             ACTION: IncreaseY\n\
             OUTCOME: (2, 1)\n");
    }
}