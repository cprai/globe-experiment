//! Propagation formulations (spec §2/§3): Cowell is the default and the
//! correctness oracle. KS regularization joins as a sibling at refactor
//! P7 - the `Formulation` trait abstraction lands with that second
//! implementation, not before.

pub(crate) mod cowell;
