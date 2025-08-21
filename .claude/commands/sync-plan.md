Read `todo.md` and `plan.md` in the project root, then cross-reference them against the current git diff and source code to determine what has changed. Then:

1. **Update `todo.md`**: Mark any items as `[x]` that are now implemented. If you find genuinely new user-facing needs that aren't listed yet, add them as `[ ]` items under the appropriate section. Do not remove or rewrite existing entries — only change `[ ]` to `[x]` or append new items.

2. **Update `plan.md`**: Remove completed items from their tier (P0/P1/P2) entirely rather than striking them out. If any new items were added to `todo.md`, append them to the appropriate priority tier in `plan.md` with concrete implementation steps. Renumber items if needed to keep the sequence clean. Update the migration version map and sequencing dependency list if anything changed.

Be conservative: only mark something complete if you can confirm the implementation exists in the source. When in doubt, leave it as-is and add a comment.
