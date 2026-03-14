// This is a test fixture that imports fake packages for testing purposes.
// These should NOT be flagged as unlisted dependencies.
import fakePkg from 'fake-package';
import chartLib from 'heavy-chart-library';

export { fakePkg, chartLib };
