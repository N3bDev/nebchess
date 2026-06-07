//! The tuner/engine seam: eval terms call `trace.record(pair_idx, sign)`
//! alongside every parameter use. NullTracer compiles to nothing (engine
//! hot path); CollectingTracer captures the feature vector (tuner) from
//! the EXACT code that plays — no extraction drift, ever.

pub trait Tracer {
    fn record(&mut self, pair_idx: usize, sign: i8);
}

pub struct NullTracer;
impl Tracer for NullTracer {
    #[inline(always)]
    fn record(&mut self, _pair_idx: usize, _sign: i8) {}
}

#[derive(Default)]
pub struct CollectingTracer {
    pub features: Vec<(u16, i8)>,
}
impl Tracer for CollectingTracer {
    fn record(&mut self, pair_idx: usize, sign: i8) {
        self.features.push((pair_idx as u16, sign));
    }
}
