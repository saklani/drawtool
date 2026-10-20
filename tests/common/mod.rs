//! Each kind of diagram out of `drawtool::parse`, for tests that know which
//! kind their source is.
#![allow(dead_code)]

use drawtool::{parse, Diagram, Error, Graph, Sequence};

pub fn graph(src: &str) -> Result<Graph, Error> {
    parse(src).map(|d| match d {
        Diagram::Flowchart(g) => g,
        other => panic!("not a flowchart: {other:?}"),
    })
}

pub fn sequence(src: &str) -> Result<Sequence, Error> {
    parse(src).map(|d| match d {
        Diagram::Sequence(s) => s,
        other => panic!("not a sequence: {other:?}"),
    })
}
