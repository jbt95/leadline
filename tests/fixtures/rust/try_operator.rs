fn parse(text: &str) -> Result<i32, std::num::ParseIntError> {
    let value = text.trim().parse::<i32>()?;
    Ok(value + 1)
}
