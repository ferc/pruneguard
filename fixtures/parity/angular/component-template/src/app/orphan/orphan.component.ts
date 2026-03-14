import { Component } from '@angular/core';

@Component({
  selector: 'app-orphan',
  template: '<div>Not declared in any module</div>',
  standalone: true,
})
export class OrphanComponent {}
