export const previewContentRequest = (id: number, createdAt: number) =>
  ["get_clipboard_content", { id, createdAt }] as const;
