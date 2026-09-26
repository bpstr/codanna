export function isolatedCalendarToken(): number {
    return 1;
}

export function readBrowserPreference(): boolean {
    return window.matchMedia("(prefers-reduced-motion: reduce)").matches;
}

export function calendarWeekStart(): number {
    return 1;
}

export function readCalendarWeekStart(): number {
    return calendarWeekStart();
}

export function displayCalendarWeekStart(): number {
    return readCalendarWeekStart();
}
