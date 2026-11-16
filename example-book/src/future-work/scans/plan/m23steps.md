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
  - id: "Quick"
    title: "The exit's quick passes"
    status: done
    needs: ["Counted"]
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
  - id: "Rounds"
    title: "The exit's rounds on the lab"
    status: done
    needs: ["I346", "I347", "O146", "O148"]
  - id: "Decided"
    title: "Its open clauses decided with the user"
    status: done
    needs: ["Rounds"]
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
  - id: "Cell"
    title: "A cell whose feed does not wrap inside an arm"
    status: done
    needs: ["Decided"]
  - id: "I351"
    title: "Item 351: steady inserts fall 75 s into an arm"
    status: done
    needs: ["Cell"]
    docs: ["../../../appendix/resolved/the-cells-fall-is-the-drives.md"]
  - id: "Again"
    title: "The exit's rounds again, a control beside them"
    status: done
    needs: ["I353f", "I351"]
  - id: "Clauses"
    title: "Their read and step clauses decided with the user"
    status: done
    needs: ["Again"]
---
# M23's steps and its exit

Imported from the mermaid diagram at line 1160 of `whats-left-todo.md` by `mdbook-reef import`.
