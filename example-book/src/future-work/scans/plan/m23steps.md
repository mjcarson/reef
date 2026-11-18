---
id: "m23steps"
title: "M23's steps and its exit"
group: "scans-plan"
needs: ["M22", "w1", "X17", "X17b", "X18", "Q62"]
nodes:
  - id: "F123"
    title: "F123 A node's walk and scan counters"
    status: done
    docs: ["../../../features/walk-and-scan-counters.md"]
  - id: "F124"
    title: "F124 A scan's page planned on the loop"
    status: done
    docs: ["../../../features/scan-page-planned.md"]
  - id: "F125"
    title: "F125 Archive reads in runs, in flight"
    status: done
    needs: ["F124"]
    docs: ["../../../features/page-reads-in-runs.md"]
    acceptance:
      - unit: A page's archive reads are cut into runs in disk order
        status: met
      - unit: A run never spans two archives
      - integration: { text: "A scan whose runs are in flight together still answers in order", test: "tests/scan_runs.rs" }
      - feature: Reads in flight are bounded by the node's read budget
        status: met
  - id: "F126"
    title: "F126 Encodes in steps"
    status: done
    needs: ["F124"]
    docs: ["../../../features/encodes-in-steps.md"]
  - id: "F127"
    title: "F127 Folds in steps, the check at the end"
    status: done
    needs: ["F125", "F126"]
    docs: ["../../../features/folds-in-steps.md"]
  - id: "F128"
    title: "F128 The pace in device time"
    status: done
    needs: ["F125"]
    docs: ["../../../features/device-time-pace.md"]
  - id: "F129"
    title: "F129 The drivers on the rebuilt walk"
    status: done
    needs: ["F127"]
    docs: ["../../../features/drivers-on-the-rebuilt-walk.md"]
  - id: "Counted"
    title: "The workspace run and the counts"
    status: done
    needs: ["F123", "F128", "F129"]
    description:
      why: "M23's seven steps rebuild the walk every scan and driver takes, so the whole workspace must pass with them, and its tests be counted, before the exit's lab passes."
      what: "The workspace run at `d0102e69` counts 2,934 tests with F123 to F129 and item 345: 2,933 pass, and the fixture row that failed has its wait fixed by the counting change."
  - id: "Quick"
    title: "The exit's quick passes"
    status: done
    needs: ["Counted"]
    description:
      why: "M23's eight exit rounds judge figures that a defect in the walk, its pace or the bench could skew, and a short pass finds such defects before the rounds are spent."
      what: "Two quick passes over a tenth of `Big` each found four things the exit's figures depended on, items 346 and 347, O146 and O148, each fixed before the rounds."
  - id: "I346"
    title: "Item 346: paced reads go to their device together"
    status: done
    needs: ["Quick"]
    docs: ["../../../appendix/resolved/paced-reads-alone.md"]
  - id: "I347"
    title: "Item 347: encode steps bounded by rows"
    status: done
    needs: ["Quick"]
    docs: ["../../../appendix/resolved/encode-steps-by-rows.md"]
  - id: "O146"
    title: "O146 Sixteen scan reads a device"
    status: done
    needs: ["Quick"]
    docs: ["../../../appendix/optimizations.md#o146-four-scan-reads-a-device-held-a-scan-alone-to-two-thirds-of-its-rate"]
  - id: "O148"
    title: "O148 A shed stream's wait doubled"
    status: done
    needs: ["Quick"]
    docs: ["../../../appendix/optimizations.md#o148-a-scan-driver-asked-a-shed-page-again-every-2-ms-beside-pages-paced-for-tens"]
    description:
      what: "A scan stream shed twice waits 2 ms, doubling up to 64 ms until a page is answered: sheds fall to about 2 a page, a Zen1 node's cpu over a scan alone from 0.68 cores to 0.52."
  - id: "Rounds"
    title: "The exit's rounds on the lab"
    status: done
    needs: ["I346", "I347", "O146", "O148"]
    description:
      why: "M23 can't close until its exit is judged on the lab: the cell beside a whole scan held to W7's budget, and a scan alone to W12's rate and cost."
      what: "M23's exit ran eight rounds on the lab: the read clause held, with no eviction pass and the drivers' figures no worse, and four clauses were left for the user to decide."
  - id: "Decided"
    title: "Its open clauses decided with the user"
    status: done
    needs: ["Rounds"]
    description:
      why: "The exit's rounds left four clauses open, the insert, step, cost and rate clauses, each the user's to read on the record's evidence before M23 could go on."
      what: "Each is decided: the rounds run again on a cell that does not wrap, item 350 fixed in M23, W12's cost read above the idle node, and the share kept at 75 ms."
  - id: "Count"
    title: "F130 A loop's steps counted by kind, their own cpu beside"
    status: done
    needs: ["Decided"]
    docs: ["../../../features/loop-steps-by-kind.md"]
  - id: "I350"
    title: "Item 350: steps of large resident partitions"
    status: done
    needs: ["Count"]
    docs: ["../../../appendix/resolved/encode-copy-deadline.md"]
  - id: "I353"
    title: "Item 353: light steps past 100 µs, attributed"
    status: done
    needs: ["I350"]
    docs: ["../../../appendix/resolved/capture-allocations.md"]
  - id: "I353f"
    title: "Item 353's fix: a capture timed by its parts, bounded"
    status: done
    needs: ["I353"]
    description:
      why: "Item 353 found the light steps past 100 µs were `Big`'s long captures' own work, so M23's step clause couldn't be read until those captures were bounded."
      what: "A capture is timed by its parts, every node's allocator hands freed pages back at 100 ms, and a capture allocates each collection once, about halving the Zen1 hosts' long `Big` captures."
  - id: "Cell"
    title: "A cell whose feed does not wrap inside an arm"
    status: done
    needs: ["Decided"]
    description:
      why: "The first exit's cell ran out of new rows inside an arm and wrapped into overwrites, so W7's insert clause couldn't be read over a whole arm."
      what: "The rounds again run on a cell of 1,100,000 rows whose feed never wraps inside an arm, at 6 GiB a node, so the insert clause is read over the whole arm."
  - id: "I351"
    title: "Item 351: steady inserts fall 75 s into an arm"
    status: done
    needs: ["Cell"]
    docs: ["../../../appendix/resolved/the-cells-fall-is-the-drives.md"]
  - id: "Again"
    title: "The exit's rounds again, a control beside them"
    status: done
    needs: ["I353f", "I351"]
    description:
      why: "The decisions of 2026-10-31 left the insert and step clauses to be read on the exit's rounds run again, once item 350 was fixed and the cell no longer wrapped."
      what: "Eight rounds with four controls ran on 2026-11-03: the insert clause held over the whole arm, and the read clause beside `Big` and the step clause missed, left to the user."
  - id: "Clauses"
    title: "Their read and step clauses decided with the user"
    status: done
    needs: ["Again"]
    description:
      why: "The rounds again missed the read clause beside `Big` at 1.14 times and the step clause on remainders placed after M23, so M23 couldn't close until the user decided them."
      what: "M23 closes on 2026-11-04: the read clause held against a twin whose scan was paced to almost nothing, and the step clause met with its remainders apart, each after M23."
---
# M23's steps and its exit

Imported from the mermaid diagram at line 1160 of `whats-left-todo.md` by `mdbook-reef import`.
