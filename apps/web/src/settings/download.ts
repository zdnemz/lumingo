/**
 * Hands a text file to the browser's download. The file is built here from
 * what the program sent, in a `blob:` address of this page's own origin, so no
 * request goes anywhere.
 */
export function saveTextFile(filename: string, text: string): void {
  const url = URL.createObjectURL(new Blob([text], { type: "application/json" }));
  const link = document.createElement("a");
  link.href = url;
  link.download = filename;
  link.rel = "noopener";
  document.body.append(link);
  link.click();
  link.remove();
  URL.revokeObjectURL(url);
}
