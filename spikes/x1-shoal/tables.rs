// X1's one table, shared by the node program and the app through `include!`
//
// A client refuses a server whose schema fingerprint differs from its own, so both halves are
// built from the same rows. Each includer writes its own imports and its own `#[shoal::db]`
// struct, since only that attribute differs between the server and the client half.

/// A note the spike writes and reads back to prove a round trip
#[derive(
    Debug, Clone, PartialEq, Archive, Serialize, Deserialize, ShoalUnsortedTable, DeepSizeOf,
)]
#[rkyv(derive(Debug))]
#[shoal_table(db = "X1")]
pub struct Note {
    /// The id this note is partitioned by
    #[shoal(partition)]
    pub id: u64,
    /// The note's text
    pub text: String,
}
