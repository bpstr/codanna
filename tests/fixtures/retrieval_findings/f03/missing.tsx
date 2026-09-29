import { lazy } from 'react';
const MissingCalendar = lazy(() => import('./missing-provider').then(module => ({ default: module.Calendar })));
export function MissingPicker() { return <MissingCalendar />; }
