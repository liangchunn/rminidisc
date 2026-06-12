struct Query(Vec<u8>);

// From<<M as TryInto<Query>>::Error>

fn scan<M>(message: M) -> anyhow::Result<Query>
where
    M: TryInto<Query>,
    anyhow::Error: From<M::Error>,
{
    match message.try_into() {
        Ok(v) => Ok(v),
        Err(e) => Err(From::from(e)),
    }
}

impl TryFrom<&str> for Query {
    type Error = anyhow::Error;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        todo!()
    }
}

#[test]
fn t() {
    let z = scan("0010");
}
