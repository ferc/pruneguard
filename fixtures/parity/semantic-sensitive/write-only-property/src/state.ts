export class AppState {
  private _count = 0;
  private _lastError: string | null = null;

  get count(): number {
    return this._count;
  }

  set count(value: number) {
    this._count = value;
  }

  get lastError(): string | null {
    return this._lastError;
  }

  set lastError(value: string | null) {
    this._lastError = value;
  }
}
