import { lazy } from 'react';
const Calendar = lazy(() => import('unindexed-package').then(module => ({ default: module.Calendar })));
export function ExternalPicker() { return <Calendar />; }
