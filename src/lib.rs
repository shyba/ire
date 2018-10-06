#![allow(unknown_lints)]
#![cfg_attr(all(test, feature = "nightly"), feature(test))]

//! An I2P router implementation in Rust.

#[macro_use]
extern crate arrayref;
#[macro_use]
extern crate futures;
#[macro_use]
extern crate lazy_static;
#[macro_use]
extern crate log;
#[macro_use]
extern crate nom;

extern crate aes;
extern crate block_modes;
extern crate byteorder;
extern crate bytes;
extern crate cookie_factory;
extern crate data_encoding;
extern crate flate2;
extern crate i2p_snow;
extern crate itertools;
extern crate num_bigint;
extern crate num_traits;
extern crate rand;
extern crate sha2;
extern crate signatory;
extern crate signatory_dalek;
extern crate signatory_ring;
extern crate siphasher;
extern crate tokio_codec;
extern crate tokio_executor;
extern crate tokio_io;
extern crate tokio_tcp;
extern crate tokio_timer;

#[cfg(test)]
#[macro_use]
extern crate pretty_assertions;
#[cfg(test)]
extern crate tempfile;
#[cfg(all(test, feature = "nightly"))]
extern crate test;

mod constants;
pub mod crypto;
pub mod data;
pub mod i2np;
mod netdb;
pub mod router;
pub mod transport;

#[cfg(test)]
mod tests;
