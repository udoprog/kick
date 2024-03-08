pub(crate) mod document {
    use musli::{Context, Decoder, Encoder};
    use toml_edit::Document;

    pub(crate) fn encode<C, E>(doc: &Document, cx: &C, encoder: E) -> Result<E::Ok, C::Error>
    where
        C: ?Sized + Context,
        E: Encoder<C>,
    {
        let string = doc.to_string();
        encoder.encode_string(cx, &string)
    }

    pub(crate) fn decode<'de, C, D>(cx: &C, decoder: D) -> Result<Document, C::Error>
    where
        C: ?Sized + Context,
        D: Decoder<'de, C>,
    {
        decoder.decode_string(
            cx,
            musli::utils::visit_owned_fn("a document", |cx: &C, string: &str| {
                string.parse().map_err(cx.map())
            }),
        )
    }
}
