//! plan-10: offline NNUE trainer for NebChess (bullet, CUDA). Not part of the engine build.
//! Arch: (768 -> HIDDEN)x2 -> 1, SCReLU, perspective. Quantization QA/QB/SCALE per the net contract.
//! Usage: nebchess-trainer --data <shuffled.bin> --id <net-id> --superbatches <N> [--bps <batches_per_superbatch>]
use std::env;

use bullet::{
    game::inputs::Chess768,
    nn::optimiser::AdamW,
    trainer::{
        save::SavedFormat,
        schedule::{TrainingSchedule, TrainingSteps, lr, wdl},
        settings::LocalSettings,
    },
    value::{ValueTrainerBuilder, loader},
};

const HIDDEN: usize = 768;
const QA: i16 = 255;
const QB: i16 = 64;
const SCALE: i32 = 400;

fn main() {
    let mut data = String::new();
    let mut id = "net".to_string();
    let mut superbatches = 25usize;
    let mut bps = 6104usize;
    let argv: Vec<String> = env::args().skip(1).collect();
    let mut i = 0;
    while i < argv.len() {
        let flag = argv[i].clone();
        match flag.as_str() {
            "--data" => { i += 1; data = argv.get(i).cloned().unwrap_or_default(); }
            "--id" => { i += 1; if let Some(v) = argv.get(i) { id = v.clone(); } }
            "--superbatches" => { i += 1; superbatches = argv.get(i).and_then(|s| s.parse().ok()).unwrap_or(superbatches); }
            "--bps" => { i += 1; bps = argv.get(i).and_then(|s| s.parse().ok()).unwrap_or(bps); }
            other => eprintln!("trainer: ignoring {other}"),
        }
        i += 1;
    }
    assert!(!data.is_empty(), "pass --data <path-to-shuffled.bin>");

    let mut trainer = ValueTrainerBuilder::default()
        .dual_perspective()
        .optimiser(AdamW)
        .inputs(Chess768)
        .save_format(&[
            SavedFormat::id("l0w").round().quantise::<i16>(QA),
            SavedFormat::id("l0b").round().quantise::<i16>(QA),
            SavedFormat::id("l1w").round().quantise::<i16>(QB),
            SavedFormat::id("l1b").round().quantise::<i16>(QA * QB),
        ])
        .loss_fn(|output, target| output.sigmoid().squared_error(target))
        .build(|builder, stm, ntm| {
            let l0 = builder.new_affine("l0", 768, HIDDEN);
            let l1 = builder.new_affine("l1", 2 * HIDDEN, 1);
            let stm_h = l0.forward(stm).screlu();
            let ntm_h = l0.forward(ntm).screlu();
            l1.forward(stm_h.concat(ntm_h))
        });

    let schedule = TrainingSchedule {
        net_id: id,
        eval_scale: SCALE as f32,
        steps: TrainingSteps {
            batch_size: 16_384,
            batches_per_superbatch: bps,
            start_superbatch: 1,
            end_superbatch: superbatches,
        },
        // 0.2 weight on game-result => eval-dominant (~lambda 0.8 on eval). Tunable.
        wdl_scheduler: wdl::ConstantWDL { value: 0.2 },
        lr_scheduler: lr::StepLR { start: 0.001, gamma: 0.3, step: (superbatches * 2 / 3).max(1) },
        save_rate: superbatches.max(1),
    };

    let settings = LocalSettings { threads: 4, test_set: None, output_directory: "checkpoints", batch_queue_size: 64 };
    let data_loader = loader::DirectSequentialDataLoader::new(&[data.as_str()]);

    trainer.run(&schedule, &settings, &data_loader);
    println!("done -> checkpoints/{}-{}/quantised.bin", schedule.net_id, superbatches);
}
