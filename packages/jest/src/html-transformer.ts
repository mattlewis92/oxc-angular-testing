// Transforms imported `.html` files (component `templateUrl` targets) into an
// ESM module exporting the raw template string. Pair with the main transformer
// in jest config (the ESM preset wires this up):
//
//   transform: {
//     '^.+\\.tsx?$': '@oxc-angular-testing/jest',
//     '^.+\\.html$': '@oxc-angular-testing/jest/html-transformer',
//   }
export function process(sourceText: string): { code: string } {
  return { code: `export default ${JSON.stringify(sourceText)};` };
}

// jest unwraps the transformer module's default export.
export default { process };
