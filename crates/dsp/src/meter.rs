//! Peak metering with clip detection.

#[derive(Clone, Copy, Debug, Default)]
pub struct PeakMeter {
    pub peak: [f32; 2],
    pub clipped: bool,
}

impl PeakMeter {
    /// Measures a block; `clipped` latches when any sample reaches 0 dBFS.
    pub fn measure(&mut self, l: &[f32], r: &[f32]) {
        let mut pl: f32 = 0.0;
        let mut pr: f32 = 0.0;
        for (&a, &b) in l.iter().zip(r) {
            pl = pl.max(a.abs());
            pr = pr.max(b.abs());
        }
        self.peak = [pl, pr];
        if pl >= 1.0 || pr >= 1.0 {
            self.clipped = true;
        }
    }
}
