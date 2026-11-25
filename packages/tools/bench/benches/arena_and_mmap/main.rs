mod crate_codecs;

use std::hint::black_box;
use std::mem::size_of;

use crate_codecs::{
    open_musli_mmap_full_verify, open_musli_mmap_lazy, open_rkyv_mmap_unchecked,
    open_rkyv_mmap_validated, random_read_musli_mmap, random_read_rkyv_mmap, read_serde_json,
    to_rkyv_workspace, to_serde_tasks, write_musli_workspace, write_rkyv_workspace,
    write_serde_json,
};
use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use nve_bench::arena_mmap::frame_arena::{
    recommended_frame_arena_capacity, run_arena_frames, run_heap_frames,
};
use nve_bench::arena_mmap::reloc::{write_reloc_workspace, MmapRelocWorkspace, VerifyMode};
use nve_bench::arena_mmap::workspace_arena::{
    mutate_task_in_place, populate_workspace_arena, random_read_workspace_arena, workspace_tasks,
    ArenaTask, WorkspaceRoot,
};
use nve_bench::arena_mmap::{
    deterministic_mutation_indices, deterministic_read_indices, generate_workspace_tasks,
    random_read_owned_tasks, TempFileGuard,
};
use nve_engine::{FrameArena, MmapOffsetArena};

fn bench_workload_a_build_and_write(c: &mut Criterion) {
    let mut group = c.benchmark_group("workload_a/build_and_write");
    for &(label, count) in &[("1k", 1_000usize), ("10k", 10_000usize)] {
        let tasks = generate_workspace_tasks(count);
        let serde_tasks = to_serde_tasks(&tasks);
        let rkyv_ws = to_rkyv_workspace(&tasks);
        group.throughput(Throughput::Elements(count as u64));

        let json_file = TempFileGuard::new(&format!("bw_serde_json_{label}"), "json");
        group.bench_with_input(
            BenchmarkId::new("serde_json_encode_write", label),
            &serde_tasks,
            |b, t| b.iter(|| black_box(write_serde_json(json_file.path(), black_box(t)))),
        );

        let rkyv_file = TempFileGuard::new(&format!("bw_rkyv_{label}"), "rkyv");
        group.bench_with_input(
            BenchmarkId::new("rkyv_encode_write", label),
            &rkyv_ws,
            |b, ws| b.iter(|| black_box(write_rkyv_workspace(rkyv_file.path(), black_box(ws)))),
        );

        let musli_file = TempFileGuard::new(&format!("bw_musli_{label}"), "musli");
        group.bench_with_input(
            BenchmarkId::new("musli_zerocopy_build_write", label),
            &tasks,
            |b, t| b.iter(|| black_box(write_musli_workspace(musli_file.path(), black_box(t)))),
        );

        let reloc_file = TempFileGuard::new(&format!("bw_reloc_{label}"), "nvereloc");
        group.bench_with_input(
            BenchmarkId::new("reloc_build_write", label),
            &tasks,
            |b, t| {
                b.iter(|| {
                    black_box(write_reloc_workspace(reloc_file.path(), black_box(t)).unwrap())
                })
            },
        );

        let arena_file = TempFileGuard::new(&format!("bw_arena_{label}"), "nvearena");
        group.bench_with_input(
            BenchmarkId::new("mmap_arena_populate", label),
            &tasks,
            |b, t| {
                b.iter(|| {
                    let arena = populate_workspace_arena(
                        arena_file.path(),
                        1,
                        "studio-bench",
                        black_box(t),
                        4096,
                    )
                    .unwrap();
                    black_box(arena.used_bytes())
                });
            },
        );
    }
    group.finish();
}

fn bench_workload_a_cold_load(c: &mut Criterion) {
    let mut group = c.benchmark_group("workload_a/cold_load");
    for &(label, count) in &[("1k", 1_000usize), ("10k", 10_000usize)] {
        let tasks = generate_workspace_tasks(count);
        let serde_tasks = to_serde_tasks(&tasks);
        let rkyv_ws = to_rkyv_workspace(&tasks);
        group.throughput(Throughput::Elements(count as u64));

        let json_file = TempFileGuard::new(&format!("cl_serde_json_{label}"), "json");
        write_serde_json(json_file.path(), &serde_tasks);

        let rkyv_file = TempFileGuard::new(&format!("cl_rkyv_{label}"), "rkyv");
        write_rkyv_workspace(rkyv_file.path(), &rkyv_ws);

        let musli_file = TempFileGuard::new(&format!("cl_musli_{label}"), "musli");
        write_musli_workspace(musli_file.path(), &tasks);

        let reloc_file = TempFileGuard::new(&format!("cl_reloc_{label}"), "nvereloc");
        write_reloc_workspace(reloc_file.path(), &tasks).unwrap();

        let arena_file = TempFileGuard::new(&format!("cl_arena_{label}"), "nvearena");
        {
            let arena = populate_workspace_arena(
                arena_file.path(),
                1,
                "studio-bench",
                &tasks,
                4096,
            )
            .unwrap();
            arena.sync_all().unwrap();
        }

        group.bench_function(BenchmarkId::new("serde_json_read_decode", label), |b| {
            b.iter(|| black_box(read_serde_json(black_box(json_file.path())).len()));
        });

        group.bench_function(BenchmarkId::new("rkyv_mmap_validated", label), |b| {
            b.iter(|| {
                let (m, n) = open_rkyv_mmap_validated(black_box(rkyv_file.path()));
                black_box((m.len(), n))
            });
        });

        group.bench_function(BenchmarkId::new("rkyv_mmap_unchecked", label), |b| {
            b.iter(|| {
                // SAFETY: Fixture file was created by `write_rkyv_workspace`.
                let (m, n) = unsafe { open_rkyv_mmap_unchecked(black_box(rkyv_file.path())) };
                black_box((m.len(), n))
            });
        });

        group.bench_function(BenchmarkId::new("musli_mmap_full_verify", label), |b| {
            b.iter(|| {
                let (m, n) = open_musli_mmap_full_verify(black_box(musli_file.path()));
                black_box((m.len(), n))
            });
        });

        group.bench_function(BenchmarkId::new("musli_mmap_lazy_header", label), |b| {
            b.iter(|| {
                let (m, n) = open_musli_mmap_lazy(black_box(musli_file.path()));
                black_box((m.len(), n))
            });
        });

        group.bench_function(BenchmarkId::new("reloc_mmap_full_verify", label), |b| {
            b.iter(|| {
                let ws = MmapRelocWorkspace::open(black_box(reloc_file.path()), VerifyMode::Full)
                    .unwrap();
                black_box(ws.view().tasks().len())
            });
        });

        group.bench_function(BenchmarkId::new("reloc_mmap_header_only", label), |b| {
            b.iter(|| {
                let ws =
                    MmapRelocWorkspace::open(black_box(reloc_file.path()), VerifyMode::HeaderOnly)
                        .unwrap();
                black_box(ws.view().tasks().len())
            });
        });

        group.bench_function(BenchmarkId::new("reloc_mmap_unchecked", label), |b| {
            b.iter(|| {
                // SAFETY: Fixture file was created by `write_reloc_workspace`.
                let ws = unsafe { MmapRelocWorkspace::open_unchecked(black_box(reloc_file.path())) }
                    .unwrap();
                black_box(ws.view().tasks().len())
            });
        });

        group.bench_function(BenchmarkId::new("mmap_arena_reopen", label), |b| {
            b.iter(|| {
                let arena = MmapOffsetArena::open(black_box(arena_file.path())).unwrap();
                black_box(workspace_tasks(&arena).unwrap().len())
            });
        });
    }
    group.finish();
}

fn bench_workload_a_random_read(c: &mut Criterion) {
    let mut group = c.benchmark_group("workload_a/random_read_1000");
    group.throughput(Throughput::Elements(1_000));

    for &(label, count) in &[("1k", 1_000usize), ("10k", 10_000usize)] {
        let tasks = generate_workspace_tasks(count);
        let rkyv_ws = to_rkyv_workspace(&tasks);
        let indices = deterministic_read_indices(count, 1_000);
        let expected = random_read_owned_tasks(&tasks, &indices);

        let rkyv_file = TempFileGuard::new(&format!("rr_rkyv_{label}"), "rkyv");
        write_rkyv_workspace(rkyv_file.path(), &rkyv_ws);
        let (rkyv_mmap, _) = open_rkyv_mmap_validated(rkyv_file.path());
        assert_eq!(random_read_rkyv_mmap(&rkyv_mmap, &indices), expected);

        let musli_file = TempFileGuard::new(&format!("rr_musli_{label}"), "musli");
        write_musli_workspace(musli_file.path(), &tasks);
        let (musli_mmap, _) = open_musli_mmap_lazy(musli_file.path());
        assert_eq!(random_read_musli_mmap(&musli_mmap, &indices), expected);

        let reloc_file = TempFileGuard::new(&format!("rr_reloc_{label}"), "nvereloc");
        write_reloc_workspace(reloc_file.path(), &tasks).unwrap();
        let reloc_ws = MmapRelocWorkspace::open(reloc_file.path(), VerifyMode::Full).unwrap();
        assert_eq!(
            reloc_ws.view().random_read_checksum(&indices).unwrap(),
            expected
        );

        let arena_file = TempFileGuard::new(&format!("rr_arena_{label}"), "nvearena");
        let arena = populate_workspace_arena(
            arena_file.path(),
            1,
            "studio-bench",
            &tasks,
            4096,
        )
        .unwrap();
        assert_eq!(random_read_workspace_arena(&arena, &indices).unwrap(), expected);

        group.bench_function(BenchmarkId::new("owned_vec_in_ram", label), |b| {
            b.iter(|| black_box(random_read_owned_tasks(black_box(&tasks), black_box(&indices))));
        });

        group.bench_function(BenchmarkId::new("rkyv_mmap_zero_copy", label), |b| {
            b.iter(|| black_box(random_read_rkyv_mmap(&rkyv_mmap, black_box(&indices))));
        });

        group.bench_function(BenchmarkId::new("musli_mmap_zero_copy", label), |b| {
            b.iter(|| black_box(random_read_musli_mmap(&musli_mmap, black_box(&indices))));
        });

        group.bench_function(BenchmarkId::new("reloc_mmap_zero_copy", label), |b| {
            b.iter(|| {
                black_box(
                    reloc_ws
                        .view()
                        .random_read_checksum(black_box(&indices))
                        .unwrap(),
                )
            });
        });

        group.bench_function(BenchmarkId::new("mmap_arena_zero_copy", label), |b| {
            b.iter(|| {
                black_box(random_read_workspace_arena(&arena, black_box(&indices)).unwrap())
            });
        });
    }
    group.finish();
}

fn bench_workload_a_incremental_mutate(c: &mut Criterion) {
    let mut group = c.benchmark_group("workload_a/incremental_mutate");

    for &(size_label, count, mut_counts) in &[
        ("1k", 1_000usize, [1usize].as_slice()),
        ("10k", 10_000usize, [1usize, 100usize].as_slice()),
    ] {
        let base_tasks = generate_workspace_tasks(count);

        for &mut_count in mut_counts {
            let param = format!("{size_label}_{mut_count}rec");
            let indices = deterministic_mutation_indices(count, mut_count);
            group.throughput(Throughput::Elements(mut_count as u64));

            let json_file = TempFileGuard::new(&format!("mut_serde_{param}"), "json");
            let mut serde_tasks = to_serde_tasks(&base_tasks);
            let mut step_j = 0u64;
            group.bench_function(BenchmarkId::new("serde_json_rewrite", &param), |b| {
                b.iter(|| {
                    step_j += 1;
                    for &idx in &indices {
                        serde_tasks[idx].status = (step_j % 5) as u32;
                        serde_tasks[idx].priority = (step_j % 4) as u32;
                        serde_tasks[idx].updated_at_ms += 1;
                    }
                    black_box(write_serde_json(json_file.path(), &serde_tasks))
                });
            });

            let rkyv_file = TempFileGuard::new(&format!("mut_rkyv_{param}"), "rkyv");
            let mut rkyv_ws = to_rkyv_workspace(&base_tasks);
            let mut step_rk = 0u64;
            group.bench_function(BenchmarkId::new("rkyv_rewrite", &param), |b| {
                b.iter(|| {
                    step_rk += 1;
                    for &idx in &indices {
                        rkyv_ws.tasks[idx].status = (step_rk % 5) as u32;
                        rkyv_ws.tasks[idx].priority = (step_rk % 4) as u32;
                        rkyv_ws.tasks[idx].updated_at_ms += 1;
                    }
                    black_box(write_rkyv_workspace(rkyv_file.path(), &rkyv_ws))
                });
            });

            let musli_file = TempFileGuard::new(&format!("mut_musli_{param}"), "musli");
            let mut musli_tasks = base_tasks.clone();
            let mut step_m = 0u64;
            group.bench_function(BenchmarkId::new("musli_rewrite", &param), |b| {
                b.iter(|| {
                    step_m += 1;
                    for &idx in &indices {
                        musli_tasks[idx].status = (step_m % 5) as u32;
                        musli_tasks[idx].priority = (step_m % 4) as u32;
                        musli_tasks[idx].updated_at_ms += 1;
                    }
                    black_box(write_musli_workspace(musli_file.path(), &musli_tasks))
                });
            });

            let reloc_file = TempFileGuard::new(&format!("mut_reloc_{param}"), "nvereloc");
            let mut reloc_tasks = base_tasks.clone();
            let mut step_r = 0u64;
            group.bench_function(BenchmarkId::new("reloc_rewrite", &param), |b| {
                b.iter(|| {
                    step_r += 1;
                    for &idx in &indices {
                        reloc_tasks[idx].status = (step_r % 5) as u32;
                        reloc_tasks[idx].priority = (step_r % 4) as u32;
                        reloc_tasks[idx].updated_at_ms += 1;
                    }
                    black_box(write_reloc_workspace(reloc_file.path(), &reloc_tasks).unwrap())
                });
            });

            let arena_file = TempFileGuard::new(&format!("mut_arena_{param}"), "nvearena");
            let mut arena = populate_workspace_arena(
                arena_file.path(),
                1,
                "studio-bench",
                &base_tasks,
                4096,
            )
            .unwrap();
            let mut step_a = 0u64;
            group.bench_function(BenchmarkId::new("mmap_arena_in_place", &param), |b| {
                b.iter(|| {
                    step_a += 1;
                    let mut last_off = 0usize;
                    for &idx in &indices {
                        last_off = mutate_task_in_place(
                            &mut arena,
                            idx,
                            (step_a % 5) as u32,
                            (step_a % 4) as u32,
                            1_800_000_000_000 + step_a as i64,
                        )
                        .unwrap();
                    }
                    black_box(last_off)
                });
            });

            let mut step_ms = 0u64;
            group.bench_function(
                BenchmarkId::new("mmap_arena_in_place_msync", &param),
                |b| {
                    b.iter(|| {
                        step_ms += 1;
                        let mut min_off = usize::MAX;
                        let mut max_end = 0usize;
                        for &idx in &indices {
                            let off = mutate_task_in_place(
                                &mut arena,
                                idx,
                                (step_ms % 5) as u32,
                                (step_ms % 4) as u32,
                                1_800_000_000_000 + step_ms as i64,
                            )
                            .unwrap();
                            min_off = min_off.min(off);
                            max_end = max_end.max(off + size_of::<ArenaTask>());
                        }
                        arena.sync_range(min_off, max_end - min_off).unwrap();
                        let root_ptr = arena.root_ptr::<WorkspaceRoot>();
                        black_box(arena.commit_root(root_ptr).unwrap())
                    });
                },
            );
        }
    }
    group.finish();
}

fn bench_workload_b_ui_tree_100_frames(c: &mut Criterion) {
    let mut group = c.benchmark_group("workload_b/ui_tree_100_frames");
    const FRAMES: u32 = 100;

    for &(label, nodes) in &[
        ("1k", 1_000usize),
        ("10k", 10_000usize),
        ("50k", 50_000usize),
    ] {
        group.throughput(Throughput::Elements((nodes as u64) * (FRAMES as u64)));

        group.bench_with_input(
            BenchmarkId::new("heap_vec_string", label),
            &nodes,
            |b, &n| {
                b.iter(|| black_box(run_heap_frames(black_box(n), FRAMES)));
            },
        );

        let mut arena = FrameArena::with_capacity(recommended_frame_arena_capacity(nodes));
        group.bench_with_input(
            BenchmarkId::new("frame_arena_bump_reset", label),
            &nodes,
            |b, &n| {
                b.iter(|| black_box(run_arena_frames(&mut arena, black_box(n), FRAMES)));
            },
        );
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_workload_a_build_and_write,
    bench_workload_a_cold_load,
    bench_workload_a_random_read,
    bench_workload_a_incremental_mutate,
    bench_workload_b_ui_tree_100_frames,
);
criterion_main!(benches);
