pub(crate) mod string {
    use std::error::Error;
    use std::fmt;
    use std::str::FromStr;

    use musli::{Context, Decoder, Encoder};

    pub(crate) fn encode<T, E>(doc: &T, encoder: E) -> Result<(), E::Error>
    where
        T: fmt::Display,
        E: Encoder,
    {
        encoder.collect_string(doc)
    }

    pub(crate) fn decode<'de, T, D>(decoder: D) -> Result<T, D::Error>
    where
        T: FromStr<Err: 'static + Send + Sync + Error>,
        D: Decoder<'de>,
    {
        let cx = decoder.cx();
        decoder.decode_unsized(|string: &str| string.parse().map_err(cx.map()))
    }
}
