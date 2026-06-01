// CommonJS variant of the HTML transformer: imported `.html` files (component
// `templateUrl` targets, emitted as `require("./x.html")` in CJS mode) become a
// module whose `module.exports` is the raw template string.
export function process(sourceText: string): { code: string } {
  return { code: `module.exports = ${JSON.stringify(sourceText)};` };
}

// jest unwraps the transformer module's default export.
export default { process };
