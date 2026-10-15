// Indonesian strings for the read screens. Same keys and order as `read.en.ts`.
export const readId = {
  "read.text": "Teks bacaan",
  "read.word": "Kata dengan arti: {word}",
  "read.gloss": "{word} artinya {gloss}",
  "read.gloss.close": "Tutup artinya",

  "read.title": "Membaca bertingkat",
  "read.intro": "Pilih tingkat dan ketik apa yang ingin kamu baca. Program meminta teks beserta pertanyaannya kepada penyedia AI-mu. Kalau tidak ada teks yang bisa ditulis, set bacaan dari unitmu yang ditawarkan.",
  "read.setup.title": "Pilih tingkat dan topik",
  "read.level": "Tingkat",
  "read.level.fixed": "Teks ini pada tingkat {level}. Untuk membaca pada tingkat lain, buka lagi halaman bacaan.",
  "read.topic": "Mau membaca tentang apa",
  "read.topic.placeholder": "Sehari di pasar",
  "read.topic.hint": "Cukup beberapa kata. Teksnya ditulis untuk tingkat di atas.",
  "read.start": "Minta teks",
  "read.starting": "Meminta teks",
  "read.again": "Teks lain",

  "read.fallback.title": "Teks dari unitmu",
  "read.reason.provider_unavailable": "Penyedia AI tidak dapat dijangkau, jadi tidak ada teks baru yang ditulis. Teks berikut sudah ditulis dan tidak memerlukan penyedia.",
  "read.reason.unusable": "Dua teks dari penyedia tidak lolos pemeriksaan, jadi keduanya tidak ditampilkan. Teks berikut sudah ditulis dan tidak memerlukan penyedia.",
  "read.empty.title": "Belum ada bacaan untuk tingkat ini",
  "read.empty.body": "Tidak ada unit pada tingkat ini yang punya set bacaan, dan tidak ada teks yang ditulis. Tidak ada yang dikarang untuk mengisi kekosongan itu. Coba tingkat lain, atau siapkan penyedia AI.",
  "read.set.from": "Dari unit {unit}",

  "read.generated.label": "Teks buatan model",
  "read.generated.note": "Model yang menulis teks ini, jadi bisa ada rincian yang keliru.",
  "read.generated.regenerated": "Teks pertama gagal satu pemeriksaan, jadi ini teks kedua.",
  "read.generated.vocabulary.skipped": "Pemeriksaan kosakata dilewati: belum ada daftar kata yang terpasang.",

  "read.score": "{correct} dari {total} benar",
  "read.score.note": "Latihan saja: skor ini tidak dihitung untuk perkiraan tingkat.",
} as const;
