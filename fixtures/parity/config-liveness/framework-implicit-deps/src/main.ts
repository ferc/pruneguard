// react-dom is the DOM renderer for React. It is required by every React
// app but never directly imported in user code with modern JSX transforms.
import React from 'react';

export function App() {
  return React.createElement('div', null, 'Hello');
}
