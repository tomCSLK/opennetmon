-- Language of the user interface per user (`de`, `en`); NULL follows the browser (`Accept-Language`).
ALTER TABLE users ADD COLUMN language TEXT;
