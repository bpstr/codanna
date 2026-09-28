import { lazy } from 'react';
export function ShadowedWrapperPicker(lazy: typeof import('react').lazy) {
  const LocalCalendar = lazy(() => import('./provider').then(module => ({ default: module.Calendar })));
  return <LocalCalendar />;
}
