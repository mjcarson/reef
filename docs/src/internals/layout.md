# The layout

The layout is layered (Sugiyama-style), done group by group from the inside out. Each open group
is laid out on its own and then becomes one box in its parent's layout. An arrow between groups
is laid out at the innermost level that holds both ends. On each level:

- back edges are reversed to break cycles;
- boxes are layered by longest path, with sources pulled down to just above what they feed;
- long edges get dummy boxes;
- barycenter sweeps order each layer, keeping the order with the fewest crossings;
- weighted isotonic regression places the boxes along each layer, which keeps their order and
  gaps and straightens long edges;
- boxes no arrow joins are laid out apart: each connected part is placed on its own, then set
  just past the parts before it on every layer they share, so a box with no arrows sits beside
  the rest rather than where it first fell.

A group with no arrows among its members is packed in a grid.
