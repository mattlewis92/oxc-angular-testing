export function realFn(): string {
  return 'real';
}

const dep = { realFn };
export default dep;
