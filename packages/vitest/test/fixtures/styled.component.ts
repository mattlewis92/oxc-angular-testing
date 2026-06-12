import { Component } from '@angular/core';

@Component({
  selector: 'app-styled',
  template: '<div class="styled"></div>',
  styleUrl: './styled.component.scss',
  styles: ['h1 { color: red; }'],
})
export class StyledComponent {}
