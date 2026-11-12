//! The network side of a scan: what each request is for, which auction a
//! bid request belongs to and whether its answer came in time, and what an
//! OpenRTB exchange says.

pub mod classify;
pub mod exchange;
pub mod openrtb;
pub mod rows;
