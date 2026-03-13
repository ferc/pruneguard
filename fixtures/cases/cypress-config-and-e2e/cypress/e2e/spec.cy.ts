describe("Home page", () => {
  it("loads successfully", () => {
    cy.visit("/");
    cy.get("h1").should("be.visible");
  });
});
