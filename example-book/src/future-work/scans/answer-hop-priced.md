# X41. A scan answer's hop between shards, priced

**Reported** (run 2026-11-15). This is the record of spike [X41](spikes.md#x41-a-scan-answers-hop-between-shards-priced):
what a page's answer costs its node when it is written by a shard that did not read it, against the same bytes written
by the shard that did. [W6](distribution.md#streams) left handing a scan's connection to the executor it reads as a
later gain, measured first: a connection lands on whichever shard accepted it, so a page read on one shard is written
by another unless the two are one.

**The verdict: measured, not taken.** Written after a hop, an answer cost 1.06 times the cpu a gibibyte of one written
where it was filled on europa, and 1.07 times on titan. That is 0.02% and 0.07% of a core at the default link share's
31.25 MB a second, far from both of the rule's bounds. The lab's spike build was not made, and the hand-off is not built.

## The question

**What would change the design**, written before the run ([the spike's row](spikes.md#x41-a-scan-answers-hop-between-shards-priced)):
the hand-off is built only if hopped answers cost at least 1.25 times the cpu a gibibyte of answers written where they
were read, and the difference is at least 2% of a Zen1 core at the default link share, or a scan's rate on europa's
loopback, unpaced, is bound by the shard writing its answers. The method was amended by the *After M24* plan's review
before any run: priced in process first, and the lab's spike build made only if the in-process figure lies past the
rule's bounds. The predictions were written first (`shoal-spike/results/x41/predictions.md`).

## How it is run

`shoal-spike hop` (`shoal-spike/src/hop.rs`): two glommio executors, each pinned to a core of its own. A host fills a
4 MiB body, the most a page's answer asks, writing every byte as an encode would. It then either writes the body to a
loopback connection of its own (`here`) or hands it over a kanal channel, as shards hand each other pages, to a
writer that writes it to its own connection (`hop`). The body goes as sixteen 256 KiB data frames, each behind a
32-byte head in one vectored write, to a sink on a third core that reads and drops it. The writing thread's own cpu
is read around the writes, and on the hop's side the host's handing over and the writer's taking are read too.
Unpaced, so every byte goes as fast as the loopback takes it: the bytes `here` writes are as warm as they will ever
be, the hop's best case to lose against. Eight rounds of a gibibyte a side, the sides' order alternating.

- **europa** (Ryzen 9 7945HX): the host on cpu 10, the writer on 11, the sink on 14, cores of one CCD.
- **titan** (Ryzen Embedded V1756B, Zen1): built for znver1 and copied there; the host on cpu 2, the writer on 4, the
  sink on 6, each on a core of its own. Its tmdb node was stopped and its governor at `performance` for the run, both
  put back after it (`shoal-spike/results/x41-host.sh`).

## The figures

| Host | Here, ms of cpu a GiB | Hop, ms of cpu a GiB | The hop | Its share of a core at 31.25 MB/s |
| --- | --- | --- | --- | --- |
| europa | 125.6 [124.9, 130.0] | 133.5 [131.2, 156.7] | 1.063 times | 0.02% |
| titan | 326.8 [323.6, 336.8] | 350.5 [336.5, 366.8] | 1.072 times | 0.07% |

The hop's own handing over and taking, the wake across cores among it, is 6 to 7 ms of cpu a gibibyte on europa and
11 to 13 on titan, about 45 µs a body of 4 MiB on titan; most of what the hop costs. The writes alone cost the same on
both sides of a round when the hop went second in it and 7% more when it went first, an order effect inside the
spread rather than a cost of the bytes' core.

The full tables, round by round, are `shoal-spike/results/x41/europa.md` and `titan.md`.

## Against the predictions

| Prediction | Held? |
| --- | --- |
| 1. The hop 1.0 to 1.1 times a write where the body was filled, on both hosts | **Held**: 1.063 and 1.072 |
| 2. The hop's own handing over and taking under a millisecond of cpu a GiB | **Failed**: 6 to 13 ms a GiB, about 25 to 45 µs a body, the wake across cores |
| 3. The difference under 0.5% of a Zen1 core at 31.25 MB a second | **Held**: 0.07% |

## What it says for the design

- **A 4 MiB answer is past either core's L2**, so the writer reads it from the L3 they share wherever it was filled,
  and the hop's bytes cost what bytes written in place cost.
- **What a hop costs is a wake a page**, tens of microseconds of a gibibyte's hundreds of milliseconds.
- **The hand-off is not built**: F84's release and adopt of a stream, a reserved byte of a client's hello and the
  relay's state rebuilt on its adopter would buy 0.07% of a Zen1 core at the default share.

## Limitations

- **In process, not on a node**: a real page's answer is written by the coordinating shard's write relay between the
  loop's other work, and its bytes were encoded a step at a time, perhaps while other pages' bytes passed through the
  same caches. The rule's in-process bound was chosen so the lab need not be asked for less than a clear answer.
- **Unpaced**: at the default share a 4 MiB answer takes about 130 ms to leave, long enough for its bytes to leave
  every cache whichever core writes them, which makes the hop cost less, not more.
- **Plaintext**: X11 found a hop under kTLS within its noise, since the kernel's encryption reads the bytes once more.

## Related

- [X41 on the spikes page](spikes.md#x41-a-scan-answers-hop-between-shards-priced), and [W6](distribution.md#streams).
- [X11](../../object-storage/streamed-bodies.md#4-a-connection-handed-over-and-bytes-that-hop), which priced an inbound
  stream's hop.
- [F134](../../features/walked-records-uncopied.md), why the hop copies nothing.
- [The *After M24* box](whats-left-todo.md#finishing-after-m24).
