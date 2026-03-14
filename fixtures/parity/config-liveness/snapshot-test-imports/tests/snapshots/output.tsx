// This is a code-splitter test snapshot output.
// It contains synthetic imports that should not be flagged.
import { useMemo } from 'tan-react';
import { createApp } from 'fake-framework';

export const App = () => useMemo(() => createApp());
