// Minimal stand-in for `react/jsx-runtime` so the TSX integration test needs no
// real React. Returns the element shape the automatic runtime produces.
function jsx(type, props) {
  return { type, props };
}
module.exports = { jsx, jsxs: jsx, Fragment: 'Fragment' };
