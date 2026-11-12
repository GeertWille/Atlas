//! Every request of the visit: the page's, and those of the iframes and
//! workers below it. `assembly` turns the browser's events into records,
//! `targets` knows which session belongs to what, `fields` reads the events'
//! parts. The recorder carries out what the assembler asks for.

pub(crate) mod assembly;
pub(crate) mod fields;
pub(crate) mod targets;
