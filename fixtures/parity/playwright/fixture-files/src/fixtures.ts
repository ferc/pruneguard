import { test as base } from '@playwright/test';
import { createAuthFixture } from './auth-fixture';

export const test = base.extend({
  authenticatedPage: createAuthFixture(),
});
