const service = require("./service");

module.exports = function run(batch) {
  try {
    return batch.map((items) => service.priceOrder(items, "US"));
  } catch (error) {
    return [];
  }
};
