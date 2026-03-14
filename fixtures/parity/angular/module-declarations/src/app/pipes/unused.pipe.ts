import { Pipe, PipeTransform } from '@angular/core';

@Pipe({ name: 'unused' })
export class UnusedPipe implements PipeTransform {
  transform(value: string): string {
    return value.toUpperCase();
  }
}
