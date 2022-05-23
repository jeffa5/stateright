//! This module provides code to define and verify the correctness of
//! an object or system based on how it responds to a collection of potentially
//! concurrent (i.e. partially ordered) operations.
//!
//! # Defining Correctness Via A Reference Implementation
//!
//! [`SequentialSpec`] is a trait for defining correctness via a "reference implementation" (e.g.
//! "*this system should behave like a queue*").  Stateright includes reusable implementations such
//! as [`register`] for register-like semantics and [`vec`] for stack-like semantics.  Implementing
//! the trait yourself is also straightforward -- just define two `enum`s for invocations and
//! returns. Then associate these (as [`SequentialSpec::Op`] and [`SequentialSpec::Ret`]
//! respectively) with a state type that implements [`SequentialSpec::invoke`].
//!
//! # Verifying Concurrent System Implementations
//!
//! A concurrent system can be verified against an implementation of the [`SequentialSpec`] trait
//! by using a [`ConsistencyTester`] for an expected [consistency model],  such as
//! [`LinearizabilityTester`]. In that case, operations are sequential (think blocking I/O) with
//! respect to an abstract thread-like caller, which is identified by a distinct "thread ID"
//! (sometimes called a "process ID" in the literature, but these are assumed to be
//! single-threaded, so thread ID arguably provides better intuition for most developers).
//!
//! An [`actor::Id`] for instance can serve as a thread ID  as long as the
//! actor does not invoke a second operation in parallel with the first. This
//! restriction only exists for the purposes of verifying consistency semantics,
//! and actors need not sequence their operations to a concurrent
//! system outside of that use case. Alternatively, a single actor could maintain
//! its own [`ConsistencyTester`] with local thread IDs for multiple concurrent
//! invocations.
//!
//! # Additional Reading
//!
//! For more background on specifying the semantics of concurrent systems, see
//! publications such as:
//!
//! - ["Consistency in Non-Transactional Distributed Storage
//!   Systems"](http://vukolic.com/consistency-survey.pdf) by Viotti and Vukolić
//! - ["Principles of Eventual
//!   Consistency"](https://www.microsoft.com/en-us/research/publication/principles-of-eventual-consistency/)
//!   by Burckhardt
//! - ["Software Foundations Volume 2: Programming Language
//!   Foundations"](https://softwarefoundations.cis.upenn.edu/plf-current/index.html)
//!   by Pierce et al.
//!
//! [`actor::Id`]: crate::actor::Id
//! [consistency model]: https://en.wikipedia.org/wiki/Consistency_model
//! [`vec`]: self::vec

mod consistency_tester;
mod linearizability;
mod sequential_consistency;

pub use consistency_tester::ConsistencyTester;
pub mod register;
pub mod write_once_register;
pub use linearizability::LinearizabilityTester;
pub use sequential_consistency::SequentialConsistencyTester;
pub mod vec;

/// An implementation of this trait can serve as a sequential "reference object"
/// (in the sense of an operational specification, not a Rust reference)
/// against which to validate the [operational semantics] of a more complex
/// system, such as a distributed system.
///
/// Stateright includes the following [`ConsistencyTester`]s for verifying that a concurrent system
/// adheres to an expected [consistency model]:
///
/// - [`LinearizabilityTester`]
/// - [`SequentialConsistencyTester`]
///
/// [consistency model]: https://en.wikipedia.org/wiki/Consistency_model
/// [operational semantics]: https://en.wikipedia.org/wiki/Operational_semantics
pub trait SequentialSpec: Sized {
    /// The type of operators. Often an enum.
    type Op;

    /// The type of values return by the operators. Often an enum or
    /// [`Option`].
    type Ret: PartialEq;

    /// Invokes an operation on this reference object.
    fn invoke(&mut self, op: &Self::Op) -> Self::Ret;

    /// Indicates whether invoking a specified operation might result
    /// in a specified return value. Includes a default implementation that
    /// calls `invoke`, but a manual implementation may be provided for
    /// efficiency purposes (e.g. to avoid a `clone()` call).
    fn is_valid_step(&mut self, op: &Self::Op, ret: &Self::Ret) -> bool {
        &self.invoke(op) == ret
    }

    /// Indicates whether a sequential history of operations and corresponding
    /// return values is valid for this reference object.
    fn is_valid_history(&mut self, ops: impl IntoIterator<Item = (Self::Op, Self::Ret)>) -> bool {
        ops.into_iter()
            .all(|(op, ret)| self.is_valid_step(&op, &ret))
    }
}
