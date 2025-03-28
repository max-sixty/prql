# Claude's notes on chumsky upgrade

## Challenges upgrading from chumsky 0.9.2 to 0.10.0

The upgrade from chumsky 0.9.2 to 0.10.0 requires significant refactoring due to breaking API changes:

1. **Lifetime parameters**: Many parser functions now require explicit lifetime parameters
2. **Error trait changes**: The Error trait implementation has been modified significantly
3. **Stream and Input trait redesign**: The way input streams are handled has been completely changed
4. **Removal of parse_recovery**: Error recovery mechanisms have been redesigned

Major changes needed across multiple files:
- `prqlc/prqlc-parser/src/parser/perror.rs` - Update Error trait implementation
- `prqlc/prqlc-parser/src/parser/mod.rs` - Update all parser functions with lifetime parameters
- `prqlc/prqlc-parser/src/span.rs` - Update Span implementation for new namespace
- `prqlc/prqlc-parser/src/lexer/mod.rs` - Update lexer functions
- `prqlc/prqlc-parser/src/parser/expr.rs` - Update parser functions
- `prqlc/prqlc-parser/src/parser/stmt.rs` - Update parser functions
- `prqlc/prqlc-parser/src/parser/types.rs` - Update parser functions
- `prqlc/prqlc-parser/src/parser/interpolation.rs` - Update parser functions

Given the extensive changes required, we've decided to keep 0.9.2 for now with an explanatory comment in Cargo.toml.