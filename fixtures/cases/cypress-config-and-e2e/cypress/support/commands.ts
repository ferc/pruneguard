Cypress.Commands.add("login", (email: string, password: string) => {
  cy.visit("/login");
  cy.get("[name=email]").type(email);
  cy.get("[name=password]").type(password);
  cy.get("button[type=submit]").click();
});
