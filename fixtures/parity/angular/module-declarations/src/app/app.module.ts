import { NgModule } from '@angular/core';
import { DatePipe } from './pipes/date.pipe';
import { HighlightDirective } from './directives/highlight.directive';

@NgModule({
  declarations: [DatePipe, HighlightDirective],
  exports: [DatePipe, HighlightDirective],
})
export class AppModule {}
