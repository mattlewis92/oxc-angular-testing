import { Component, input, output } from '@angular/core';

export class Dep {
  value = 42;
}

@Component({
  selector: 'app-sig',
  template: '<p>sig</p>',
})
export class SignalComponent {
  disabled = input<boolean>(false);
  changed = output<string>();

  constructor(public dep: Dep) {}
}
