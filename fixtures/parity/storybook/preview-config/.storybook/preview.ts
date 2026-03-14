import type { Preview } from '@storybook/react';
import { withTheme } from '../src/decorators/theme-decorator';

const preview: Preview = {
  decorators: [withTheme],
  parameters: {
    layout: 'centered',
  },
};

export default preview;
